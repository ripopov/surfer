# Surfer Konata View — Software Architecture

**Status:** Proposed architecture, source-checked 2026-07-09

**Companion UX specification:** [SurferKonataUX.md](../SurferKonataUX.md)

**Primary implementation area:** `libsurfer`, with required scale work in
`ftr-parser` and remote capability work in `surver`

**Source baselines inspected:** Surfer `19ecf02`; upstream Konata
`b689fbd06a58742aaa42bd34be70bb5b63bca0a3`

This document defines the architecture for the UX in the companion
specification. It focuses on algorithms, data structures, ownership, and
integration boundaries. Type and function names are descriptive rather than a
frozen API, and detailed code is intentionally omitted.

## 1. Architecture decision

The Konata view should be a first-class Surfer tile backed by a shared,
immutable **pipeline projection**, not a special mode of the waveform renderer
and not a second copy of the FTR object graph.

The projection has four layers:

1. A parser-neutral transaction query facade owned by `TransactionContainer`.
2. A compact, paged `PipelineStore` shared by every Konata and table tile that
   references the same source/generator pair.
3. Per-tile serialized view state plus small, non-serialized runtime caches.
4. A viewport-driven renderer whose cost is bounded by visible rows, visible
   stages, or output pixels—not total trace size.

```text
FTR file / bytes / Surver
          │
          ▼
TransactionContainer query facade
          │  sequential records + relations + dictionary ids
          ▼
PipelineIndexBuilder ── publishes immutable snapshots ──► PipelineStore
          │                                                   │
          │                                                   ├─ Konata tiles
          │                                                   ├─ transaction/event tables
          │                                                   ├─ find and statistics jobs
          │                                                   └─ comparison/alignment
          ▼
quality/progress deltas ──► Surfer Message loop ──► repaint/state persistence
```

This keeps source ownership, focus, cursors, markers, themes, tiling, state
files, and commands in Surfer. Only the pipeline-specific projection, layout,
and rendering are new.

## 2. Source-grounded feasibility

### 2.1 Useful Surfer seams already exist

| Required capability | Existing source seam | Architectural use |
|---|---|---|
| Multiple local or remote sources | [`SourceId`, `SourceStore`, and source-qualified refs](../libsurfer/src/source.rs) | A pipeline key always includes `SourceId`; cross-file IDs cannot collide. |
| Transaction ownership | [`DataContainer::Transactions` and `TransactionContainer`](../libsurfer/src/data_container.rs) | `TransactionContainer` remains the facade; pipeline storage is not placed in UI code. |
| FTR event convention | [`EventIndex`](../libsurfer/src/transaction_events.rs) and [FTR_EVENTS.md](development/FTR_EVENTS.md) | Existing pairing and violation semantics are the compatibility oracle for the new builder. |
| Shared focus | [`SourceTransactionRef` and `WaveData::focused_transaction`](../libsurfer/src/wave_data.rs) | A focused instruction or stage uses the same source-qualified FTR identity as waveform and tables. |
| Async result dispatch | [`Message`](../libsurfer/src/message.rs), [`perform_work`](../libsurfer/src/async_util.rs), and the table-cache revision/cancellation pattern | Index, search, density, and statistics results use generation and revision checks before publication. |
| Serializable tiles | [`SurferPane`, `SurferTileTree`](../libsurfer/src/tiles.rs), and table tile state in [`UserState`](../libsurfer/src/state.rs) | Add a Konata pane and mirror the table split between serialized state and runtime state. |
| Virtualized tables | [`TableModel`](../libsurfer/src/table/model.rs), batched materialization, and lazy search probes | Pipeline tables and statistics reuse the table UI without materializing display strings for every row. |
| Themes and global animations | [`SurferConfig` and `SurferTheme`](../libsurfer/src/config.rs) | Pipeline palette tokens extend the theme; global animation disable is the reduced-motion authority. |
| Shared time navigation | Existing cursor, marker, viewport, and time-formatting code | Konata keeps its own two-dimensional transform but converts its X coordinate to the shared trace-time domain. |

The current FTR work has already removed several former blockers: timestamps
are `u64`, attribute names and string values share `Arc<str>`, and relations
are stored once with source/sink indexes. Those improvements should be
preserved.

### 2.2 Current paths that cannot meet the target

The following are not criticisms of the existing waveform/table feature set;
they are scale boundaries exposed by the new UX:

- [`TransactionContainer::load_stream`](../libsurfer/src/transaction_container.rs)
  loads an entire stream and then rebuilds `EventIndex` over all loaded
  transactions. Several UI paths invoke this synchronously.
- `EventIndex` keeps two transaction lookup hash maps plus event and parent
  maps. Rebuilding and duplicating those maps is costly at millions of rows
  and tens of millions of stages.
- The FTR parser's block directory currently retains only offset and
  compression state. It discards block time bounds and uncompressed size,
  which prevents informed prefetch and progress accounting.
- Parsed attributes share string bytes through `Arc<str>` but no longer retain
  their original dictionary IDs. The paged query backend must keep those IDs
  so projection building does not need a second string-to-ID hash pass.
- File-backed FTR initially skips transaction blocks, but byte-backed FTR
  eagerly parses all bodies. A large downloaded trace therefore lacks the
  same lazy behavior as a local file.
- The waveform transaction renderer builds hash maps of draw commands and
  scans `TxGenerator::transactions` using time-order assumptions. The Konata
  convention requires recorded begin order to remain stable even when times
  move backward.
- The current transaction and event table models build owned display and
  search strings for every row. That is deliberately convenient for ordinary
  tables, but is an avoidable multi-gigabyte multiplier at pipeline scale.
- `TransactionId`, `GeneratorId`, and related FTR IDs use `usize`. The on-disk
  format uses unsigned integer IDs; the in-memory identity must be `u64` on
  both native and wasm targets.
- On wasm, the current `perform_work` abstraction does not move CPU-heavy work
  off the browser UI thread. Cooperative slicing or a Web Worker backend is
  required for smooth large-trace behavior.
- [`surver`](../surver/src/server.rs) currently exposes Wellen hierarchy,
  timetable, and signal payloads. It does not advertise or serve transaction
  pages, so scaled remote FTR is a protocol addition, not an already available
  path.

Consequently, a renderer built directly on the current `Vec<Transaction>` can
deliver an early functional milestone, but **multi-million-instruction support
must not be claimed until paged parsing/querying and background loading land**.

### 2.3 What is being beaten in original Konata

The comparison is against observed implementation choices, not against the
JavaScript language in the abstract:

- Upstream [`op_list.js`](https://github.com/shioyadan/Konata/blob/b689fbd06a58742aaa42bd34be70bb5b63bca0a3/op_list.js)
  stores object-rich operations in several resolution levels, serializes pages
  to JSON, gzip-compresses them, and synchronously gunzips/parses a page on a
  cache miss. Its own comment estimates roughly 1 KiB per expanded operation.
- Deep zoom-out selects every Nth operation. It bounds work by sampling, but
  can hide short stalls and flushes.
- Upstream [`konata_renderer.js`](https://github.com/shioyadan/Konata/blob/b689fbd06a58742aaa42bd34be70bb5b63bca0a3/konata_renderer.js)
  walks visible operations, creates gradients, issues stage-by-stage Canvas 2D
  calls, and walks the rows again for dependencies.
- Upstream search in
  [`store.js`](https://github.com/shioyadan/Konata/blob/b689fbd06a58742aaa42bd34be70bb5b63bca0a3/store.js)
  reconstructs one complete string per operation and scans sequentially,
  yielding periodically to the event loop.
- Statistics scan operations sequentially and yield in large batches.

Surfer's design replaces those with compact columns, aggregate rather than
sampled LOD, asynchronous page decode, batched geometry, and parallel or
cooperatively sliced analysis. Performance wins still require measurements;
the acceptance gates in this document prevent architecture intent from being
mistaken for a benchmark result.

## 3. Performance contract

### 3.1 Invariants

These rules are more important than any particular container type:

1. No UI-frame operation is proportional to total instruction or stage count.
2. No pan, zoom, hover, or focus operation synchronously reads, decompresses,
   sorts, or rebuilds a trace page.
3. The detailed renderer visits only visible rows and their visible stages.
4. The density renderer is proportional to contributing rows plus output
   pixels on a cache miss, runs off the UI thread, and is constant-time to draw
   when cached.
5. Search, statistics, transitive dependency walks, and index construction are
   cancellable and publish progress.
6. Text and arbitrary attributes remain interned or paged; the view never
   creates a permanent concatenated search string per row.
7. Cache sizes are explicit budgets. They do not grow with navigation history.
8. Progressive publication never renumbers an instruction that has already
   been published.

### 3.2 Acceptance budgets

These are test targets, not current performance claims. Results must record
hardware, OS, build, backend, trace shape, and cold/warm cache state.

| Measure | Target gate |
|---|---|
| Warm pan/zoom frame on desktop | p95 UI CPU below 8 ms and p99 below 16.7 ms at 60 Hz |
| Input to changed frame | p95 below 50 ms while background jobs are active |
| UI-thread blocking work | No task above 8 ms in normal interaction; no synchronous page decode |
| First useful local view | Header, rows, and a provisional viewport appear before the complete pipeline index |
| Resident projection directory | Design budget at or below 64 bytes per instruction, excluding shared dictionary and paged detail |
| Encoded stage detail | Design budget at or below 24 bytes per normal stage before arbitrary attributes, with overflow records paid only when needed |
| Decoded detail | Bounded LRU; starting budgets 256 MiB native and 64 MiB web, configurable after measurement |
| Density/render caches | Bounded independently; starting budget 64 MiB native and 24 MiB web |
| Correctness under zoom-out | No sampled-row data loss; every output bin reports the range and count it aggregates |

Benchmark tiers should include:

- The checked-in 4,041-instruction / 51,961-stage trace.
- 100,000 instructions for routine profiling.
- 1,000,000 instructions with at least ten stages per instruction.
- A multi-million-instruction stress trace large enough to force decoded-page
  eviction.
- Adversarial traces with non-monotonic time, extremely long stages, many
  overlapping lanes, dense dependencies, duplicate RIDs, and all rows matching
  a search.

## 4. Identity and normalized semantics

### 4.1 Pipeline key

A shared projection is identified by:

- source ID;
- parent stream and generator IDs;
- matching events generator ID;
- source cache generation;
- pipeline-convention/projection version.

This key lets several tiles share one build and makes reload invalidation
identical to table-cache invalidation.

### 4.2 Stable row identity

`PipelineRowId` is the zero-based position of the parent transaction in the
parent generator's **recorded begin order**. It is dense and never derived from
timestamp, RID, SID, filter position, or current load progress.

Each row also retains its source-qualified FTR transaction identity. The two
identities serve different jobs:

- row ID provides compact array indexing and the user-visible ID;
- source transaction identity provides cross-view focus, reload resolution,
  event-table activation, and exact trace provenance.

Out-of-order timestamps set a quality bit; they never reorder rows.

### 4.3 Typed normalization

The builder recognizes convention fields by interned attribute-name ID and
then validates their `DataType`. It never calls general display formatting in
the hot build loop.

| Concept | Normal form |
|---|---|
| label/detail/stage name | dictionary/string ID; missing is an explicit sentinel |
| SID and RID | optional `u64` |
| thread ID | optional interned typed key, so numeric and textual producers remain distinguishable |
| flushed | tri-state: true, false, unknown |
| lane | interned typed key mapped to a compact lane ID in first-seen order |
| time | raw `u64` trace tick; invalid ranges retain both raw endpoints and a flag |
| transaction/event locator | compact ordinal within its generator plus the original `u64` FTR ID |
| attributes | typed scalar or interned value referenced through an attribute span |

Unknown/mistyped fields remain inspectable as raw attributes and add a quality
diagnostic. Normalization does not rewrite the trace.

### 4.4 Clock mapping

Geometry is always stored and computed in trace ticks. A pipeline clock is a
view projection with positive period and optional signed origin. Cycle `n`
maps to the half-open trace interval defined by the UX specification.

Current `TxGenerator` metadata contains no clock mapping. Therefore the
resolver order is:

1. pipeline-convention metadata, once the FTR producer/parser exposes it;
2. a persisted per-tile user override;
3. trace-time mode.

The view must not infer a period from observed fetch spacing. The ruler uses
integer/rational arithmetic for cycle labels and converts to floating point
only after subtracting the visible origin, avoiding precision loss at large
timestamps.

The current generator directory also has no schema or pipeline marker. Entry
point discovery may use the sibling-name pair from `EventIndex`, exactly as the
UX permits, but semantic validation remains provisional until attributes and
`parent_of` relations have been inspected.

Current FTR relations also carry no endpoint timestamps. Dependency endpoint
resolution therefore normally begins at the configured execution stage and
then transaction start. The higher-priority relation-timestamp rule becomes
active only if a future relation representation actually supplies it.

## 5. `PipelineStore`: compact shared data

### 5.1 Ownership and snapshots

One source-level registry in `SystemState` owns a build entry per pipeline key.
An entry contains the latest `Arc<PipelineSnapshot>`, progress, error state,
revision, cancellation token, and consumer count.

A snapshot is immutable. Publication clones only a small directory of page
`Arc`s, not row or stage payloads. Readers never lock the builder. Old
snapshots remain valid until the frame or worker using them completes.

The builder may mutate private tail buffers, but only complete immutable pages
and a copied tail snapshot cross into UI state. Updates are coalesced to at
most one message per display frame; the first usable page is published
immediately.

### 5.2 Resident row directory

The row directory is a structure-of-arrays, split into fixed-size row pages.
A starting page size of 4,096 rows balances cache locality, cheap page
replacement, and low top-level overhead; benchmarks may tune it.

Resident columns include:

- parent transaction ID and generator ordinal;
- start and end tick;
- optional SID, RID, and thread key;
- label and detail string IDs;
- flushed/retired/quality flag words;
- lane count;
- detail-page locator;
- page-level min start, max end, counts, and quality summaries.

Columns that are entirely absent from a trace are omitted. Optional numeric
columns use a presence bitset plus a dense value column instead of sentinel
values when that saves memory.

The page directory also contains a segment tree over row pages with min-start
and max-end summaries. Time-window queries can prune row ranges without
assuming monotonic timestamps.

### 5.3 Paged detail

Stages, arbitrary attributes, and non-parent dependency edges are stored in
immutable detail pages. Pages are independently decodable and evictable.

Within a decoded page:

- stage records use columnar arrays;
- a CSR offset array maps each row to its stage span;
- stages are stable-sorted by `(start tick, recorded event order)`;
- common start offsets and durations use 32-bit row-relative values, with an
  overflow side table for large values;
- name and lane are compact IDs;
- flags cover zero duration, invalid range, outside-parent range, unnamed,
  and multiple parentage;
- an event locator preserves exact stage focus and table navigation;
- attribute spans point into typed attribute columns;
- outgoing and incoming dependency edges use CSR adjacency arrays.

No row owns a heap vector, string, or hash map. The CSR representation makes a
normal row lookup two offsets and one contiguous scan.

Local derived pages may be cached in the OS cache directory, keyed by source
fingerprint, source generation, parser version, and convention version. A
sidecar is an optimization only: it is never written beside the trace without
an explicit product decision, and deleting it cannot change semantics.

### 5.4 Lookup indexes

Lookup structures are selected from data shape rather than always using hash
maps:

- Dense transaction IDs use a direct ordinal vector; sparse IDs use sorted
  `(ID, row)` pairs with binary search.
- Per-thread dense RIDs use a vector plus duplicate/missing bitsets. Sparse
  RIDs use sorted pairs.
- SID lookup uses the same density test and records duplicates explicitly.
- Event transaction ID to stage locator uses a direct or sorted index.
- Relation-name and stage-name indexes use dictionary IDs, never strings.

The builder records which representation it chose so memory reports and
benchmarks remain explainable. Hashing is reserved for incremental tail state
and bounded temporary joins, not one heap allocation per final record.

### 5.5 Visibility and Y layout

Four common row layouts are pre-indexed:

1. all rows, one unit per instruction;
2. non-flushed rows, one unit per instruction;
3. all rows, natural split-lane units;
4. non-flushed rows, natural split-lane units.

Each row page stores local prefix sums for those modes; the top-level page
directory stores cumulative totals. Fixed-height split lanes uses the
one-unit index. This gives:

- logical Y to row in two binary searches;
- row to logical Y from page and local prefix values;
- hide-flushed and lane-mode toggles without rebuilding an N-element filtered
  vector;
- stable scroll anchoring when the layout changes.

Rows with `flushed == unknown` remain in the non-flushed projection, matching
the UX requirement not to guess commitment.

## 6. Ingestion and progressive loading

### 6.1 Parser-neutral input

`TransactionContainer` should expose a query/streaming facade for:

- stream and generator metadata;
- block metadata and progress;
- sequential transactions for selected generators;
- sequential relations with recorded order;
- typed attributes and dictionary access;
- point lookup by transaction locator.

The first adapter wraps today's in-memory `FTR`, which enables an early tile
without duplicating convention logic. The scale backend reads FTR blocks on
demand. Both feed the same normalizer and must produce byte-for-byte equivalent
projection snapshots for the same trace.

Existing direct consumers of `generator.transactions` can be migrated to the
facade incrementally. `TransactionContainer` remains the integration boundary
seen by `DataContainer`, `WaveData`, and tables.

### 6.2 Required FTR block directory

The file scan should retain one `BlockMeta` per transaction or relation chunk:

- file offset and encoded length;
- compressed/uncompressed state and uncompressed length;
- stream ID;
- declared start/end tick where available;
- recorded chunk ordinal;
- parse/validation status.

This metadata is small, makes progress honest, enables prioritization, and
allows local random access without loading transaction bodies.

### 6.3 Build algorithm

The bounded-memory build uses three logical passes. For small traces an
implementation may fuse them, but observable order and diagnostics must match.

1. **Discover and publish parent rows.** Scan selected parent/event generator
   records in file order. Assign row IDs only when a parent transaction is
   first encountered. Extract resident columns and publish complete row pages.
   Stage records are written to temporary page runs keyed by event transaction
   ID.
2. **Resolve relations.** Stream relation chunks in recorded order. Filter
   `parent_of` relations whose sink belongs to the selected events generator,
   and instruction-to-instruction relations for dependencies. Join them to
   parent rows/event records. Large joins use sorted runs and merge joins;
   bounded incremental hash tables are allowed only for active tail pages.
3. **Canonicalize detail pages.** Group stages and dependency edges by parent
   row, stable-sort each row's short stage span, emit CSR pages, compute
   quality summaries, and atomically replace provisional detail pages. Build
   overview summaries and final lookup indexes.

An event that precedes its parent or relation is held in a bounded pending run,
not dropped. If memory pressure is reached, the run spills to the derived cache
rather than growing an unbounded hash map.

The UI may therefore show rows before their stages and label totals as
provisional. Stage attachment never changes row IDs.

### 6.4 Append/reload behavior

Every source replacement increments the existing source cache generation. A
new generation builds beside the old snapshot; the tile keeps showing the old
snapshot with a stale/reloading badge until the first useful new snapshot is
ready.

Restoration order is:

1. exact source transaction ID;
2. unique `(thread ID, RID)`;
3. unique SID;
4. saved timestamp and nearest recorded-order row.

For a growing file, unchanged complete pages may be reused when source
fingerprint and block directory prove their bytes unchanged. Otherwise the
reload is a complete generation replacement. A partial parse keeps all
successfully published pages and a fatal diagnostic.

## 7. View transform and navigation

### 7.1 Per-tile transform

The Konata tile needs a two-dimensional transform independent of the waveform
viewport:

- horizontal origin in trace ticks with sub-tick floating pan remainder;
- trace ticks per pixel;
- vertical origin in logical row/lane units;
- row units per pixel;
- current and target values for animation;
- a stable anchor row/time used during layout changes and reload.

The X transform converts to Surfer's shared time only at cursor, marker,
command, and cross-view boundaries. This avoids coupling several independent
Konata tiles to the waveform zoom while preserving exact time interoperability.

Pointer-centered zoom preserves the logical `(time, row unit)` beneath the
pointer. Long timestamps are converted to pixels by subtracting the viewport
origin before `f32` conversion.

### 7.2 Diagonal follow

Before a vertical movement, select the instruction nearest the viewport's
vertical center and record its fetch X position. After applying the new Y
origin, select the new center instruction and change horizontal origin by the
difference between their actual fetch timestamps. This preserves pipeline
texture across bubbles, simultaneous fetches, and variable fetch width.

Compensation is zero if either row is unavailable, both fetch timestamps are
equal, or the move clamps at an edge. Shift-modified movement skips the
horizontal update.

This improves on original Konata's top-row anchor while implementing the UX's
center-row rule.

### 7.3 Animation

Navigation stores a target transform and evaluates one short ease-out curve
from monotonic time. New input retargets the curve; it never queues animations.
The existing global animation setting and `animation_time` provide the default
and reduced-motion behavior; the Konata default is clamped to the UX's roughly
80–100 ms movement unless the user has explicitly configured another duration.

During density-mode animation, the last aggregate image is transformed as a
temporary preview and a final-target density job is scheduled. Detailed mode
rebuilds only visible geometry. Repaints are requested only while input,
animation, or a visible progress update is active.

### 7.4 Ruler ticks

Tick selection uses a 1/2/5 × power-of-ten step for time mode and an integer
step for cycle mode. The chosen step guarantees room for the widest visible
formatted label. Iteration begins at the first visible tick, so an extremely
large absolute cycle never creates work proportional to its value.

### 7.5 Interaction state

Fit-all computes X bounds from the loaded min/max stage or instruction extent
and Y bounds from the active layout prefix index, then chooses the smaller
coupled scale that fits both axes with padding. X-only and Y-only zoom commands
change one transform component; ordinary wheel/pinch zoom keeps Konata's
coupled X/Y behavior.

Transient UI is an explicit priority stack: context menu/popover, active
search, search result card, pinned tooltip, then instruction focus. `Esc` pops
one layer. Hover and pinned tooltips retain a row/stage locator rather than
formatted text; their selectable text is materialized only while visible.

Changing hide-flushed or lane layout captures the top visible instruction and
its fractional pixel offset, changes the prefix index, then solves the new Y
origin to preserve that anchor. “Adjust position” uses the nearest valid row
and its fetch tick, or the sync group's alignment anchor.

## 8. Rendering architecture

### 8.1 Frame pipeline

For each frame the tile:

1. consumes input and evaluates the current transform;
2. chooses LOD from the smaller of row height and one-cycle width;
3. maps the clipped Y range to row IDs with a small prefetch margin;
4. acquires decoded detail pages or schedules non-blocking prefetch;
5. emits detailed mesh batches or a cached density aggregate;
6. draws labels, ruler, focus/dependencies, cursor, markers, minimap, and
   quality/loading overlays;
7. performs mathematical hover/focus hit testing.

Missing pages render a low-detail row extent or placeholder. A frame never
waits for I/O or decode.

### 8.2 Detailed and strip LOD

The renderer scans the visible row range and each row's contiguous stage span.
Stages are rejected against the visible time interval before geometry is
created.

Rectangles of the same layer are accumulated into a small number of
`egui::Mesh` batches. Borders, warning glyphs, flush overlays, focus outlines,
and arrows are separate batches so toggles do not rebuild unrelated geometry.
There is no `HashMap<TransactionRef, DrawCommand>` and no egui widget per
stage.

Non-zero stages use their exact half-open tick range. Zero-duration stages use
a diamond mesh centered at the exact tick. Overlapping same-lane stages receive
deterministic insets and draw order. Alternating row backgrounds, invalid
above/below-trace regions, and out-of-parent warning outlines are simple
clipped mesh layers independent of stage count.

Text is emitted only at the text LOD. Interned stage names reuse cached text
layouts keyed by string ID, font, and scale bucket. Label-pane rows are
virtualized from the same visible row range.

For a long multi-cycle stage, trailing numbers start at the first visible
cycle cell and stop at the last visible cell. Work is bounded by screen
columns, not stage duration.

### 8.3 Density LOD

Deep zoom-out must aggregate rather than select every Nth instruction.

A density job receives a quantized viewport, output size, row-layout mode, and
palette/flush options. Rows arrive in Y order, so the job processes one output
pixel row at a time with reusable horizontal difference buffers. For every
contributing instruction extent it performs constant-time range updates into
that row's horizontal bins. Prefix sums then produce:

- total occupied-row density;
- flushed density;
- fetch/retire envelope;
- warning density;
- optional dominant stage/stall class when detail pages are resident.

The basic algorithm is `O(R + W×H)` for `R` contributing rows and output size
`W×H`, with `O(W)` counter scratch plus the output image, rather than storage
for a full counter grid or work proportional to total stage area. The row-page
segment tree prunes pages whose extents do not intersect the visible time
range. Page summaries seed a fast provisional image while an exact cold
aggregate is running; benchmark evidence, not speculation, decides whether a
deeper precomputed pyramid is worth its memory.

Results are cached by quantized transform and revision. Each bin retains its
row/time bounds and count for tooltips and click-to-zoom. The minimap uses the
same aggregator at a fixed narrow resolution and is rebuilt incrementally from
page summaries.

### 8.4 CPU-first, backend-neutral rendering

The first implementation should use egui meshes and textures. This works with
Surfer's existing native and wasm renderers, state/snapshot tests, clipping,
and themes. A custom GPU pipeline is justified only if profiles show mesh
submission or density texture upload—not indexing or text—to be the remaining
bottleneck. The store and query architecture does not depend on the rendering
backend.

### 8.5 Palette resolution

Palette lookup is integer-ID based and cached per theme/options revision:

- *Auto* assigns a stage-depth ID in global first-appearance order and maps it
  through a perceptually spaced theme palette; later progressive pages never
  renumber an existing stage.
- *Unique* hashes the interned stage-name ID to a stable palette slot.
- *Thread ID* hashes the typed thread key to hue and uses stage depth for a
  bounded lightness adjustment.
- lane identity contributes a stable hue/lightness offset only in schemes that
  distinguish lanes.
- stall-name IDs are resolved once from the case-sensitive configured set and
  override the base color with the theme's stall token.
- flat/custom schemes compile into the same `(thread, lane, stage) -> color`
  lookup table.

Focus, flush, and warning states are separate outline/overlay layers, never
color substitutions. Contrast is checked against the active canvas background
when a palette revision is built, not per stage per frame.

## 9. Hit testing, focus, and dependencies

### 9.1 Hit testing

Y maps to a row through the layout prefix index. X maps to an exact trace tick.
The row's stages are sorted by start time. A binary search finds the last
possible start; a prefix-maximum-end array allows the scan to stop once all
earlier stages end before the hit time. Typical rows remain a very short
linear scan.

Zero-duration stages receive pixel-space hit tolerance converted back to a
time interval. Overlapping hits are returned in stable lane/start/event order;
repeat-click cycling stores only the last hit key and index.

No invisible interaction rectangles or accessibility nodes are allocated for
the entire trace.

### 9.2 Shared focus

Focusing an instruction sends the existing source-qualified focus message.
Focusing a stage focuses the event transaction; the projection maps that event
back to its parent row. A tile suppresses auto-scroll for the focus message it
originated, while other tiles and tables navigate to the new identity.

The current shared focus representation is sufficient for the first
milestone. An eventual identity-only focus state would remove the cloned
`Transaction`, but is not a prerequisite for this view.

### 9.3 Dependency storage and rendering

Parent/event `parent_of` relations are excluded from dependency CSR. All other
instruction-to-instruction relations retain relation-name ID, recorded order,
and any available endpoint metadata.

Visible-arrow generation visits adjacency spans of visible rows and suppresses
edges whose peer is hidden or outside the selected LOD policy. Geometry is
batched. Focused direct neighbors require work proportional only to the focused
row's degree.

Execution-stage fallback anchors are cached per page and execution-name-set
hash. A missing fallback is represented explicitly and drawn hollow.

Producer-chain highlighting uses an iterative traversal with a row bitset,
restricted to the current in-view ancestry rule. It is cancellable, has a
revision token, and never recurses on the call stack. Stable row ID breaks
ties.

## 10. Search and statistics

### 10.1 Search corpus

The store exposes each row as a sequence of typed fragments: identities,
label, detail, stage names, and stage annotations. It does not persist one
joined `String` per row.

A search job:

1. compiles the regex before replacing the last valid result;
2. scans a circular page plan beginning just after the current row (or just
   before it for reverse search), matching the UX's wraparound order;
3. reuses one scratch buffer per worker to join a row's fragments with stable
   separators;
4. records hits in a `Vec<u64>` bitset and page hit counts;
5. publishes ordered page completions and progress;
6. checks cancellation between bounded chunks.

Next/previous uses page counts plus word-level rank/select on the result bitset,
so it does not need a worst-case vector containing every row ID. Hidden hits
remain in the bitset and are labeled hidden by the view.

Native builds scan independent pages on the existing worker/thread machinery
and merge by circular plan position. A first hit is not announced until all
earlier plan positions have completed, so parallel execution cannot change
which match is “next.” Wasm uses a Web Worker when available; the fallback
processes time-budgeted page slices and yields before the next frame. Regex
semantics are identical across executors.

The first implementation may remain a full scan because the UX explicitly
permits progressive search. A trigram index should be added only if benchmarked
search latency justifies its memory, and it may only prefilter candidates; it
must not change regex results.

### 10.2 Find-to-table

The transaction table model should read the same pipeline fragments and use
`SearchTextMode::LazyProbe`. This lets the existing table filter UI reproduce
Konata search scope, including child stage annotations, without storing a
second search corpus.

Current transaction/event table models should progressively migrate from
eager per-row strings to the transaction query facade. The existing batched
`materialize_window` method is already the correct UI boundary.

### 10.3 Statistics

Statistics are deterministic page reductions:

- instruction pages reduce fetched/committed/flush/thread counts and
  first/last relevant times;
- stage pages reduce per-name counts, clipped duration sums, and maxima;
- relation pages reduce explicit cause attribution;
- final reduction computes ratios only when required inputs are valid.

Committed means `flushed == false` with an unambiguous RID. Whole-trace elapsed
cycles span the first instruction start through the last committed end using
half-open coverage; missing clock or retirement data produces `unknown`, not a
guess. Flush attribution consumes explicit cause relation/attributes first.
The preceding-committed-instruction heuristic is a separately counted
`estimated` result and is excluded from exact rates unless requested.

Instruction classification first consumes an explicit typed class attribute,
then a named pluggable regex classifier. The result retains classifier
provenance so generic and ISA-specific heuristics never masquerade as recorded
facts.

Region statistics apply the half-open tests from the UX before accumulation.
Durations are clipped to the region. Native page reductions may run in
parallel; final combination occurs in page order to keep results reproducible.

Results use `TableModelSpec::AnalysisResults` with a pipeline-statistics kind,
reusing table sorting/filtering/copying. Cache keys include pipeline revision,
selection range, clock mapping, classifier, and estimated-attribution policy.

## 11. Surfer integration

### 11.1 Serializable state

Add `KonataTileId`, `SurferPane::Konata`, and a `konata_tiles` map beside
`table_tiles` in `UserState`.

Serialized tile state contains only durable, small data:

- source and generator pair;
- transform anchor and zoom;
- splitter position;
- clock override;
- LOD, palette, lane, flush, dependency, ruler, and minimap options;
- bookmarks;
- comparison/sync group identity and explicit alignment policy.

`SystemState` owns non-serialized `KonataRuntime` entries with animations,
hover state, decoded-page handles, search results, render caches, job tokens,
and the last observed shared focus. Closing a tile drops its runtime and
decrements its projection consumer count.

State restore remaps source IDs exactly as current table specs do. Unknown new
fields use serde defaults so old state files remain loadable.

### 11.2 Tile and message flow

`SurferTileBehavior::pane_ui` delegates to `draw_konata_tile`, matching the
table path. The tile mutates transient pointer state locally and emits semantic
messages for durable changes, shared focus/time, commands, and job lifecycle.

High-frequency motion is coalesced to one transform update per frame. Worker
messages carry pipeline key, source generation, tile/search revision where
applicable, and result. Stale results are discarded using the same defense in
depth as table cache results.

### 11.3 Hierarchy and commands

The existing generator-pair detection and transaction sidebar context menus
are the entry seam. Pair existence enables the action before bodies load;
conforming stage counts update after projection publication.

Konata command parser entries translate into messages rather than calling view
code. Command targets default to the focused Konata tile and accept explicit
tile/source qualifiers for WCP and scripts. Fuzzy generator completion reads
source-qualified transaction metadata.

### 11.4 Cursor, markers, waveform, and tables

- Ruler clicks send the existing cursor message in trace ticks.
- Cursor and markers are read from shared `WaveData` and projected into the
  tile X transform.
- Instruction/event table activation already produces a source-qualified
  focus action; the tile observes and navigates to it.
- Konata context actions open existing transaction/event tables with a filter
  or selection, adding only the pipeline-aware lazy model behavior described
  above.
- “Show in waveform” sends focus plus existing time navigation; the waveform
  view remains the owner of its viewport.

### 11.5 Comparison and synchronization

A sync group has one coordinator in runtime state and monotonically increasing
update sequence numbers, preventing message feedback loops.

RID alignment builds a per-thread match index from the compact RID indexes.
Only unique keys participate; duplicate and missing counts remain visible.
Each synchronized tile receives an anchor key, zoom, splitter width, and
relative fetch offset, then resolves those values in its own trace.

Overlay comparison stores a sorted alignment map between row IDs. Rendering
queries the two stores independently and composes their batches; it never
merges source data or assumes equal timestamps. Explicit ID/timestamp fallback
uses a different alignment policy value and is never silently selected.

### 11.6 Module boundaries

Pipeline code should live under one `libsurfer/src/konata/` module family with
separate pure model/index/layout, render, search/statistics, view, and
controller responsibilities. The dependency direction is one way:

- `ftr-parser` owns bytes, CBOR, block metadata, typed FTR records, and no UI;
- `TransactionContainer` owns parser-neutral transaction/event query semantics;
- Konata model/index/layout owns normalized pipeline data and pure algorithms,
  with no `SystemState` or egui dependency;
- Konata rendering depends on the model plus theme/egui primitives;
- the view/controller alone depends on `SystemState`, `Message`, tiles, tables,
  cursor/markers, commands, and worker lifecycle;
- `surver` owns remote production and the client owns page transport, both
  sharing a versioned wire schema that has no egui types.

This separation keeps the model benchmarkable without a GUI, lets tables use
the projection without depending on the tile, and prevents pipeline-specific
logic from leaking into the simulator-agnostic FTR parser.

## 12. Local, byte-backed, wasm, and remote sources

### 12.1 Local files

Local files use the block directory for seekable, cancellable reads. The OS
page cache and decoded-page LRU handle locality. Derived cache writes are
atomic and versioned.

### 12.2 Byte-backed files and raw URLs

Byte-backed FTR must use a cursor over shared immutable bytes and the same
block directory rather than eagerly materializing every transaction. Keeping
the downloaded encoded bytes plus only decoded visible pages prevents the
current encoded-plus-fully-expanded memory spike.

### 12.3 Wasm execution

The query/store format must contain only owned, transferable buffers and
integer IDs. A platform executor chooses:

- native worker threads / Rayon for CPU scans;
- Web Worker build and analysis with transferable page buffers;
- cooperative time-sliced fallback when workers are unavailable.

No algorithm may rely on shared-memory wasm threads. The UI receives immutable
page results through the existing message/repaint boundary.

### 12.4 Surver protocol

Scaled remote support requires explicit, versioned capabilities in Surver
status and endpoints for:

- pipeline descriptors and clock metadata;
- projection revision/progress/quality summary;
- row directory ranges;
- compressed detail pages;
- dictionary fragments;
- optional fixed-resolution overview summaries.

The wire page schema is versioned independently of Rust struct layout. Requests
include source revision and page IDs; responses are cacheable and reject stale
revisions. The client prefetches the visible page window and cancels obsolete
requests.

A legacy/raw URL may still download the complete FTR, but that path is not the
scaled Surver claim. If the server lacks transaction-page capability, the
Konata entry point is disabled with the UX-specified explanation.

### 12.5 WCP

Add Konata commands to [`surfer-wcp`](../surfer-wcp/src/proto.rs) as typed,
versioned commands. They resolve into the same messages as keyboard/menu
actions. Responses acknowledge only after validation and return stable tile or
transaction identities where the caller will need them later.

## 13. Robustness and accessibility

Quality is stored as compact row/stage bitsets plus aggregate counters. The
builder produces one diagnostic record per distinct issue location/category;
the UI does not regenerate warnings while panning.

Malformed ranges keep raw endpoints. Rendering normalizes them to an invalid
point only at projection time. Multiple parents select the first relation in
recorded order. Orphans remain accessible through raw event tables. Duplicate
RID/SID indexes retain all candidates and mark ambiguous operations
unavailable.

Accessibility exposes only visible virtualized label rows and visible/focused
stages to egui/AccessKit, with stable row IDs as semantic identity. Keyboard
navigation can address off-screen rows through commands without constructing
millions of accessibility nodes. Focus indicators, warnings, and flush state
always have non-color geometry or text.

## 14. Cache and concurrency policy

| Cache | Key | Eviction/policy |
|---|---|---|
| Shared projection | pipeline key | Reference counted; cancel build when no consumer remains |
| Decoded detail pages | pipeline revision + page ID | Byte-budget LRU; visible/focused pages pinned for the frame |
| Text layout | string ID + font/scale bucket | Bounded LRU; dictionary strings are not copied |
| Detailed render batch | tile options + quantized transform + visible page revisions | Small most-recent cache; invalidated by option or page change |
| Density aggregate | revision + quantized 2D viewport + output size + density options | Byte-budget LRU; old image may preview an animation |
| Search result | revision + regex/options | One current and one last-valid result per tile |
| Statistics | revision + range + policy | Shared result cache with cancellation and bounded entries |

The UI thread owns serialized/runtime maps and reads immutable snapshots.
Workers own builders and scratch memory. Communication is by immutable results
and atomic progress/cancellation. Lock scope never includes parsing,
decompression, regex evaluation, statistics, or geometry construction.
All implementation remains safe Rust; this design requires no new `unsafe`
blocks, `unsafe` APIs, or handwritten FFI declarations in `libsurfer`.

## 15. Testing and measurement

### 15.1 Pure algorithm tests

- recorded-order row assignment with tied/backward timestamps;
- typed attribute normalization and all missing/mistyped cases;
- event parent resolution, orphan/multiple-parent/out-of-range rules;
- direct/sorted ID indexes and duplicate detection;
- four Y-layout prefix indexes and anchor preservation;
- time/cycle transform round trips at `u64` extremes;
- pointer-centered zoom and diagonal follow;
- LOD selection at every threshold;
- density range updates against a slow reference rasterizer;
- interval hit testing with points and overlaps;
- dependency endpoint fallback and traversal tie order;
- search cancellation, wraparound, invalid regex retention, and hidden hits;
- region-statistics half-open boundaries;
- stale generation/revision result rejection;
- bounded LRU behavior.

### 15.2 Integration and snapshot tests

- open from every entry point and restore a saved Konata tile;
- checked-in Kanata sample at full, strip, and density LOD;
- focus round trips among instruction table, event table, waveform, and two
  Konata tiles;
- cursor/marker projection in cycle and time modes;
- reload preservation and stale snapshot replacement;
- malformed quality badge and filtered raw-table actions;
- side-by-side sync and overlay with unmatched/duplicate keys;
- local and mocked remote capability success/failure;
- light, dark, high-contrast, reduced-motion, and accesskit-enabled snapshots.

Snapshot fixtures must use deterministic transforms, fonts, page publication,
and animation completion. No feature relies on manual-only validation.

### 15.3 Performance harness

Add deterministic synthetic FTR generation to the existing event benchmark so
trace shape is controlled. Measure separately:

- header/block scan;
- rows-to-first-publication;
- complete projection build;
- cold and warm page decode;
- detailed geometry generation;
- density cold build and cached draw;
- pan/zoom frame latency;
- hit test;
- regex scan and cancellation latency;
- statistics;
- reload reuse;
- peak/resident memory.

Benchmark original Konata and Surfer on the same converted semantic trace where
possible. Report results; do not encode an unverified “N× faster” claim in UI
or documentation.

## 16. Delivery plan and gates

### Milestone 0 — measurement and convention lock

- Freeze the pipeline attribute/clock convention version used by the UX.
- Extend the synthetic generator to million-scale rows, stages, relations, and
  malformed cases.
- Capture current Surfer and upstream Konata baselines.

**Gate:** reproducible datasets and metrics exist before optimization claims.

### Milestone 1 — shared projection and functional tile

- Add pipeline key, registry, in-memory adapter, normalizer, immutable row/detail
  pages, and quality flags.
- Add serialized/runtime tile state and `SurferPane::Konata`.
- Implement detailed/strip LOD, virtual labels, ruler, navigation, hit testing,
  focus, cursor/markers, themes, and state restore.

**Gate:** the checked-in sample satisfies the core UX and all work per frame is
bounded by the viewport. No million-scale claim yet.

### Milestone 2 — scale foundation

- Change all FTR IDs to platform-independent `u64`.
- Add full block metadata and the transaction query facade.
- Move local and byte-backed FTR loading to background paged reads.
- Replace whole `EventIndex` rebuilds for pipeline consumers with bounded
  projection joins.
- Add decoded-page budgets, prefetch, density aggregation, minimap, and native
  plus wasm executor behavior.

**Gate:** one-million and multi-million traces meet memory and interaction
budgets while forcing page eviction. This is the first milestone allowed to
claim large-trace support.

### Milestone 3 — exploration workflows

- Dependency modes/walk/producer chain.
- Cancellable regex search and find-to-table.
- Pipeline statistics and region statistics.
- Lazy transaction/event table materialization from the shared query/store.
- Commands, bookmarks, comparison, synchronized tiles, and overlay.

**Gate:** long-running work cannot block or stale-update the UI; all UX parity
rows through local analysis are automated.

### Milestone 4 — remote and production hardening

- Surver transaction capabilities and paged protocol.
- WCP commands.
- append/reload reuse, accessibility, all malformed states, cache telemetry,
  and documented benchmark results.

**Gate:** local, bytes, wasm, and capable Surver sources have equivalent
behavior; the complete UX parity matrix is automated.

## 17. UX requirements trace

| UX area | Architectural coverage |
|---|---|
| §1 goals/principles | Performance invariants and measured gates (§3); stable identity (§4); quality preservation (§13) |
| §2 data model/clock | Typed normalization, recorded-order IDs, and explicit clock resolver (§4) |
| §3 opening | Pair-first provisional detection, hierarchy entry seam, tiles, commands (§6, §11.1–§11.3) |
| §4 view anatomy | Two-dimensional transform, virtual labels, splitter state, ruler and tile runtime (§7, §8, §11.1) |
| §5 canvas/LOD/lanes | CSR stages, prefix layouts, detailed meshes, exact density aggregation and palette resolution (§5.3, §5.5, §8) |
| §6 navigation/bookmarks | Pointer-anchored zoom, center-row diagonal follow, animation, fit/adjust, identity bookmarks (§7) |
| §7 inspection/find | Mathematical hit testing, shared focus, lazy tooltips and cancellable bitset search (§7.5, §9.1–§9.2, §10.1) |
| §8 dependencies | Dual CSR adjacency, endpoint fallbacks, batched arrows and bounded traversal (§9.3) |
| §9 color/hide/compare/minimap | Pre-indexed visibility layouts, palette compiler, alignment map/sync coordinator, density minimap (§5.5, §8.3, §8.5, §11.5) |
| §10 Surfer integration | Shared source/focus/time, pipeline-aware tables, reload generations, Surver and WCP (§11–§12) |
| §11 statistics | Deterministic page reductions, exact/estimated provenance and half-open region rules (§10.3) |
| §12 options/themes/persistence | Serialized tile state, theme-revision palettes and runtime cache keys (§8.5, §11.1, §14) |
| §13 robustness/accessibility | Quality bitsets/diagnostics, visible-only semantic nodes and automated gates (§13, §15) |
| §14 scenarios | All underlying workflows map to the shared projection, table analysis, comparison and markers (§10–§12) |
| §15 parity/beyond Konata | Upstream comparison (§2.3), milestone gates (§16), automated integration matrix (§15.2) |
| §16 UX out of scope | This document supplies the implementation architecture while leaving converter work outside the view (§1, §6) |

## 18. Rejected designs

| Design | Reason rejected |
|---|---|
| Reuse waveform transaction draw commands | Wrong Y projection, time-order assumptions, per-viewport hash maps, and no density hierarchy |
| Build one `Instruction` object with owned strings/vectors per row | Poor cache locality and memory comparable to the upstream object graph |
| Keep the current FTR graph and add another complete derived graph | Doubles the dominant memory; acceptable only as a temporary small-trace adapter |
| Rebuild `EventIndex` after every progressive batch | Repeated `O(N+E)` work and large allocation churn |
| Sort instructions by timestamp | Violates stable fetch identity and progressive loading semantics |
| Sample every Nth row at deep zoom | Fast but silently loses narrow anomalies; aggregate bins are required |
| Draw every sub-pixel stage | Overdraw grows with trace density; density rendering must be output-bounded |
| Pre-render one bitmap for the entire trace | Time/row dimensions and zoom range make it unbounded and text becomes invalid under scale |
| Synchronously decode a page on cache miss | Recreates original Konata's visible hitch; missing data must be prefetched or temporarily degraded |
| Use the waveform `Viewport` object as Konata tile state | It is one-dimensional and session-oriented; sharing conversions is useful, sharing mutable zoom state is not |
| GPU-only first implementation | Raises native/wasm/test integration risk before evidence that CPU batching is insufficient |
| Infer clock from instruction spacing | Bubbles and multi-fetch make the result semantically false |
| Treat raw-URL download as scaled remote support | It transfers and retains the whole trace and provides no page/capability semantics |

## 19. Final feasibility checklist

The architecture has a concrete source seam for every major UX area:

- tile lifecycle and persistence: `tiles.rs`, `state.rs`, `system_state.rs`;
- actions and async results: `message.rs`, `table_controller.rs`, channels;
- transaction/event normalization: `transaction_container.rs`,
  `transaction_events.rs`, `ftr-parser`;
- shared focus/time/source identity: `source.rs`, `wave_data.rs`;
- tables and analysis: `table/model.rs`, `table/cache`, existing transaction
  and event sources;
- themes and animation: `config.rs`;
- local/bytes loading: `wave_source.rs`;
- remote protocol: `surver`, `remote/client.rs`;
- scripting: `surfer-wcp`, command parser and WCP handler;
- deterministic visual testing: existing snapshot harness and the checked-in
  Kanata FTR fixture.

The only material prerequisites not already represented by an extension seam
are the paged FTR query backend, wasm off-main-thread executor, and Surver
transaction-page protocol. They are explicitly scheduled before their
corresponding scale/remote claims. No part of the target UX requires a
simulator-specific dependency in Surfer or a forked copy of the trace model.
