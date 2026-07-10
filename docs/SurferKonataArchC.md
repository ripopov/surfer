# Surfer Konata View — Software Architecture

**Status:** Architecture proposal, source-checked 2026-07-09 against Surfer
`19ecf02` and upstream Konata `b689fbd06a58742aaa42bd34be70bb5b63bca0a3`

**Companion:** [SurferKonataUX.md](../SurferKonataUX.md) (target user experience — the
requirements source for this document),
[FTR_EVENTS.md](development/FTR_EVENTS.md) (event convention),
[FtrEventsSufer.md](development/FtrEventsSufer.md) (waveform-view event UX, largely implemented)

**Primary implementation area:** `libsurfer`, with scale work in `ftr-parser` and
remote capability work in `surver`

**Scope:** algorithms, data structures, and integration strategy. Deliberately no code;
struct sketches are field inventories, not definitions.

Every integration claim in this document is anchored to a concrete type, function, or
file in the current tree (see the feasibility ledger in §15). Where the design needs
something that does not exist yet, that is stated explicitly.

---

## 1. Goals and performance targets

The UX specification defines *what* the view does. This document exists to make the
following quantitative promises implementable:

| # | Target | Value |
|---|---|---|
| G1 | Frame cost | Warm drawing is bounded by visible candidates plus explicit output budgets, never trace size. Density draw is O(screen pixels); cold page decode/construction never blocks a frame |
| G2 | View-model memory | Resident row directory targets ≤ 64 B per instruction; normal encoded stage detail targets ≤ 24 B per stage, with decoded detail and render/density caches under explicit byte budgets |
| G3 | Time to first pixels | Rows render progressively while the trace loads; no normal UI-thread task exceeds 8 ms, and no interaction synchronously reads or decompresses a page |
| G4 | Search | Asynchronous, cancellable, progressive; full regex scan of 1 M instructions in low single-digit seconds on native |
| G5 | Trace scale | 1 M and multi-million traces are P2 acceptance tiers; 10 M is a native stretch target through §6.4 without the generic object graph. Wasm capacity is measured separately within its 4 GiB address space |
| G6 | Determinism | Identical state file + trace ⇒ identical pixels, so golden-image snapshot tests cover the view |

"Beat Konata" is not aspirational hand-waving; §2 itemizes the structural costs in the
original that this design removes by construction. Per the repository rule of measured
claims, the acceptance numbers below and the phase gates in §14 are to be *recorded* by
benchmarks, not asserted.

### 1.1 Performance invariants

These invariants outrank any particular container or renderer implementation:

1. No UI-frame operation is proportional to total instruction or stage count.
2. Pan, zoom, hover, focus, and keyboard navigation never synchronously read,
   decompress, sort, or rebuild a trace page.
3. Detailed rendering visits only visible rows and their visible stages. Density
   rendering draws a cached output-sized aggregate; a cache miss is cancellable worker
   work, with page summaries providing a provisional image.
4. Search, statistics, transitive dependency walks, parsing, and index construction
   are cancellable and publish progress with source-generation and job-revision guards.
5. Text and arbitrary attributes remain interned or paged. No permanent concatenated
   search string is created per row.
6. Every cache has a byte or entry budget and cannot grow with navigation history.
7. Progressive publication never renumbers a row that has already been published.

### 1.2 Acceptance gates

These are targets, not current claims. Every result records hardware, OS, build,
backend, trace shape, and cold/warm cache state.

| Measure | Target gate |
|---|---|
| Warm pan/zoom on desktop | p95 UI CPU < 8 ms and p99 < 16.7 ms at 60 Hz |
| Input to changed frame | p95 < 50 ms while background jobs are active |
| UI-thread blocking | No normal task > 8 ms; no synchronous page decode |
| First useful local view | Header, parent rows, and a provisional viewport before the complete index |
| Resident row directory | ≤ 64 B/instruction, excluding shared dictionary and paged detail |
| Encoded stage detail | ≤ 24 B/normal stage before arbitrary attributes; overflow paid only when needed |
| Decoded-detail LRU | Initial budgets: 256 MiB native, 64 MiB web; configurable after measurement |
| Render/density caches | Initial budgets: 64 MiB native, 24 MiB web; independently bounded |
| Deep zoom-out | No sampled-row data loss; each bin retains its represented row/time bounds and count |

---

## 2. Why this design beats the original Konata

Findings from reading Konata's source (github.com/shioyadan/Konata, Electron/JS; the
claims below reflect the upstream `master` at revision `b689fbd` and its release
notes — the repository is external to this tree):

| Konata (JS) mechanism | Cost | This design |
|---|---|---|
| One heap `Op` object per instruction with nested `Stage`/`Dependency` objects; author's own estimate ~1 KB/op (`op_list.js`) | GB-scale heaps; app ships with a 64 GB V8 heap switch (`main.js`) and manual GC | Columnar row pages plus ≤ 24 B normal encoded stages (§5), no per-row heap objects |
| Ops stored **redundantly** in a 5-level decimated page store (`[1, 8, 64, 512, 4096]`), pages gzip-compressed as `JSON.stringify` blobs; cold-page touch does synchronous `gunzipSync` + `JSON.parse` on the UI thread | Multi-hundred-ms stutters on long scrolls; memory multiplied across levels | One canonical paged store. Zoomed-out reads use range summaries/aggregates (§5.6), not sampled copies, and page misses never decode on the UI thread |
| Full-canvas synchronous redraw on a 16 ms `setInterval`; a `createLinearGradient` object allocated per stage box per frame | Render cost O(visible ops × stages) with per-box allocations; animation ticks compete with parsing | Two-tier draw pipeline (cached draw commands + batched mesh painting, §7) with per-frame allocations bounded by screen size; gradients are per-vertex colors, cost zero |
| Parsing runs on the renderer thread via readline events; files ≥ 2 GB crashed the renderer before v0.36 | UI contends with the parser; hard size ceiling | Parse + model build on workers (native/Web Worker) or cooperatively sliced wasm fallback, streaming per block (§6) |
| Find = linear scan that materializes a fresh multi-line string per op, yielding via 17 ms sleeps | Search churns the compressed page store; O(trace) string allocation | Ordered page scan with reused buffers, result bitset, cancellation, and deterministic progressive hits (§9) |
| Dependency drawing fetches producers by id, potentially decompressing cold pages mid-frame | Frame spikes when arrows point far off-screen | Edges use compact CSR/draw indexes; page misses degrade and prefetch rather than synchronously decode (§5.5, §7.1) |

The structural summary: Konata pays per-object, per-frame, and per-page-decompression
costs that a columnar, aggregate-indexed, batch-rendered design simply does not have.
Every subsequent section is one of those removals made concrete.

---

## 3. Architectural overview

```text
 FTR file / shared bytes / capable Surver
        │  ftr-parser: dictionary + block directory + typed page decode
        ▼
 TransactionContainer query facade (§6.1)
        │  sequential records/relations + point lookup + dictionary ids
        ▼
 PipelineIndexBuilder ── immutable snapshots ──► KonataModel / PipelineStore (§5)
        │                                            │
        │                                            ├─ Konata tiles
        │                                            ├─ transaction/event tables
        │                                            ├─ find/statistics jobs
        │                                            └─ comparison/alignment
        ▼
 quality/progress deltas ──► Surfer Message loop ──► repaint/state persistence
                                                     │
                                                     ├─ serialized tile state
                                                     └─ bounded runtime caches
```

Four design rules govern everything below:

1. **The generic store is not the render model.** `TransactionContainer` stays the
   source/query boundary for identity and typed FTR semantics. The first adapter may
   read today's in-memory `FTR`; the scale adapter pages records without materializing
   the generic object graph. Both feed the same derived, columnar `KonataModel`. This
   is the same relationship `AnalogSignalCache` has to raw signal data
   (`libsurfer/src/analog_signal_cache.rs`): a compact, query-optimized projection
   built asynchronously and invalidated by generation counters.
2. **Per-frame work is bounded by the screen.** Every rendering regime in §7 does O(1)
   or O(log n) work per *pixel row or visible cell*, backed by indexes built once at
   load (§5.4–§5.6). No frame ever iterates all instructions.
3. **Copy the proven Surfer patterns, don't invent parallel ones.** State splitting,
   async build protocol, cache keying, cross-view coupling, command registration, and
   snapshot testing all mirror the table subsystem and analog cache — the two existing
   examples of exactly this problem shape. §10 maps each mechanism to its precedent.
4. **Storage and rendering are independent.** The logical model API is the same for
   an all-resident small trace and a paged large trace. A frame may request detail,
   but it must tolerate a miss and render row extents or summaries until the page is
   available.

---

## 4. Coordinates, identity, and the viewport

### 4.1 Row space and time space

* **Row space (Y).** A row index `r ∈ [0, n)` is the instruction's position in the
  parent generator's transaction vector — which the pipeline convention defines as
  fetch order, and which `ftr-parser` preserves as file order within
  `TxGenerator::transactions`. Row index **is** the Konata ID: stable across
  filtering, hiding, sorting, and progressive loading, satisfying UX §2. When *Hide
  flushed ops* is active, rendering operates in **visible-row space** `v ∈ [0, m)`
  translated through the visibility index (§5.7); IDs never change.
  Each row also retains the source-qualified FTR transaction identity for cross-view
  focus and reload. FTR transaction/generator/stream identities are normalized to
  `u64` on both native and wasm; current `usize` parser ids must migrate before the P2
  cross-platform scale gate.
* **Time space (X).** Native unit is the FTR tick (`u64`, as stored by the parser
  fork). The optional cycle mapping is `cycle(t) = (t − origin) div period` over
  half-open intervals; cycles are a *display projection*, all geometry and hit-testing
  math stays in ticks. Absent a mapping, the ruler falls back to trace time via the
  container's `Timescale` (note the parser's exponent convention is 4 steps per unit,
  `ftr-parser/src/types.rs::get_timescale` — the Konata view only consumes the already
  converted `TimeUnit`).

### 4.2 Clock mapping discovery

`KonataModel` carries `Option<ClockMapping{ period: u64, origin: i64 }>` per source
generator. Resolution order:

1. Convention metadata when present (whatever artifact `konata2ftr` emits — a
   metadata transaction or attribute; the convention layer owns this detection).
2. User-set value from **Set pipeline clock…**, persisted in the tile state.
3. Trace-time mode.

The view does not infer a clock from fetch or stage spacing: bubbles and multi-fetch
fronts make that estimate semantically false. Cycle labels use integer/rational
arithmetic and convert to floating point only after subtracting the visible origin.

The current generator directory has neither a schema/pipeline marker nor clock
metadata. Sibling-name pairing may therefore discover a provisional entry point, but
the model reports it as conforming only after typed attributes and `parent_of`
relations validate it. Current FTR relations also have no endpoint timestamp; the
higher-priority dependency-timestamp rule applies only when an endpoint is itself a
stage event or a future relation representation supplies one explicitly.

### 4.3 The Konata viewport

The waveform `Viewport` (`libsurfer/src/viewport.rs`) models one axis as a fraction of
total time; the Konata view needs two coupled axes and row-space semantics, so it gets
its own small `KonataViewport`:

* `left_tick: i64` + `left_frac: f64` (sub-tick pixel remainder), `px_per_tick: f64`
* `top_visible_row: f64` (fractional, in visible-row space), `row_height_px: f64`
* **Signedness matters everywhere here**: the left edge must be able to sit before
  tick 0 (free panning, pointer-anchored zoom-out near the trace start, and the very
  "scrolled off the trace body" state that *Adjust position* exists to recover from),
  and every delta — pixel mapping, diagonal compensation, drag — is a signed
  difference. All tick arithmetic is done in i64/i128 before any f64 cast; model
  timestamps (u64, bounded by the writer) convert once at the model boundary.
  Panning clamps to a defined off-trace margin (~half a viewport, matching the
  waveform viewport's edge-space behavior) rather than to tick 0.
* Pixel mapping computes `((t − left_tick) as f64) · px_per_tick` with the
  subtraction in integer space, so on-screen positions are exact regardless of
  absolute tick magnitude (no loss at 10¹²⁺ ticks).
* Zoom: half-steps of √2, ~32 of them spanning the row-height range
  [1/1024 px, 48 px] (the UX's "roughly 24" extremes; step granularity is a config
  knob). Alt-scroll, pinch, keyboard, and double-click zoom X and Y together (Konata
  behavior); Ctrl-scroll zooms only X to match Surfer's waveform panel. Both modes
  preserve the pointer's time anchor, and coupled zoom also preserves its visible-row
  anchor. Pinch zoom is continuous over the same range.
* Animation reuses the existing easing approach (`ViewportStrategy::EaseInOut`,
  viewport.rs): current + target states, eased with frame `stable_dt`, ~80–100 ms,
  instant when reduced-motion is set. While animating, the tile requests repaints —
  same reactive-repaint discipline the main canvas uses.

The viewport (and per-tile options and splitter) serializes inside
`KonataTileState`; bookmarks serialize per source generator (§8) — so `.surf.ron`
sessions restore exactly (UX §12). Bookmarks and restore anchors store *identity* —
`(FTR transaction id, tick, zoom)` — and resolve identity first, timestamp second,
per UX §6.4.

### 4.4 Typed normalization contract

The builder matches convention fields by interned attribute-name id and validates the
recorded `DataType`; it never recognizes fields through formatted display text.

| Concept | Normal form |
|---|---|
| label, detail, stage/relation name | dictionary id; missing has an explicit sentinel |
| SID and RID | optional `u64`; duplicate candidates retained |
| thread and lane | typed interned key mapped to a compact first-seen id; numeric and textual values remain distinct |
| flushed | true / false / unknown; absence is never invented as false |
| time | raw `u64` ticks; invalid ranges retain both endpoints plus a flag |
| parent/event locator | generator ordinal plus original `u64` FTR identity |
| arbitrary attributes | typed scalar or interned value behind an attribute span |

Unknown or mistyped convention fields remain inspectable as raw attributes and add a
quality diagnostic. Normalization never rewrites the trace to make it look valid.

---

## 5. The KonataModel: columnar pipeline projection

One `KonataModel` exists per pipeline key, shared by every tile, search task,
statistics job, and table adapter through `Arc<KonataModelEntry>`. The key contains
the source id, parent stream/generator id, matching events-generator id, source cache
generation, and pipeline-convention/projection version. This prevents cross-source
identity collisions and invalidates semantic as well as byte-layout changes.

**Growth vs. sharing.** A model cannot be simultaneously shared behind `Arc` and
mutated by the loader, so it is split into immutable fixed-size row pages (start with
4,096 rows and tune by benchmark), independently decodable detail pages, and a small
page directory. The builder owns private tail buffers; publication clones only a
directory of page `Arc`s and a frozen tail, never the row/stage payload. Renderers,
searches, and statistics hold a consistent snapshot while the tile runtime swaps to
the latest one. Old snapshots remain valid until their last reader finishes.

The row directory and page summaries remain resident. Stage records, arbitrary
attributes, and non-parent dependency detail are independently evictable under the
decoded-detail LRU. The all-resident adapter is merely the same interface with every
detail page pinned; it is not a second model design.

### 5.1 Instruction columns (struct-of-arrays)

| Column | Type | Purpose |
|---|---|---|
| `begin`, `end` | u64 × 2 | fetch/retire tick range |
| `tx_id` | u64 | FTR transaction id — the cross-view identity used by focus, tables, bookmarks |
| `sid` | u64 (sentinel = missing) | `insn_id_in_sim` |
| `rid` | u64 (sentinel = missing) | `retire_id` |
| `tid` | u32 typed-key ref (sentinel) | Preserves whether `thread_id` was numeric or textual; compact ids are assigned in first-seen order |
| `flags` | u16 bitset | tri-state flushed, retired, and warning bits (§12): begin-regression, event-out-of-range, unnamed-stage, negative-duration, unknown-lane, missing-rid |
| `label` | u32 dict ref | disassembly line (label pane, search) |
| `detail` | u32 dict ref (sentinel) | multi-line detail attribute |
| `detail_page`, `stage_off` | compact page/CSR locator | detail page plus offset into that page's stage store |

The resident row target is ≤ 64 B/instruction. Missing optional fields are sentinels,
never invented zeros (UX §4 ①). Text columns retain dictionary ids. The in-memory
adapter may share `Arc<str>` entries from `FTR::str_dict`; the paged adapter loads
dictionary fragments on demand. Either way, bytes exist once per distinct string and
are never copied into row/stage records. Unique disassembly/detail text is inherently
O(text), but it need not all be decoded at once.

### 5.2 Paged stage store (CSR)

Stages are logically flat and grouped by instruction through CSR, but physically live
in immutable detail pages so cold detail can be decoded and evicted independently.
Each page uses this common encoded record:

| Field | Type | Notes |
|---|---|---|
| `start_delta`, `duration` | u32 × 2 | row-relative common case; exact raw endpoints spill to an overflow table; zero duration ⇒ diamond |
| `name` | u16 | index into the stage-name table (order of first appearance — this order also drives the *Auto* color scheme) |
| `lane` | u8 | unknown lane values map to stable synthetic keys (§12) |
| `flags` | u8 | out-of-parent-range, end-before-start, unnamed, has-annotations |
| `attr_off` | u32 | offset into the annotation store |
| `event_tx` | u64 | the stage's FTR event-transaction id — identity for event-table activation ("highlight this specific stage", UX §10.2), stage-level focus, and *Move cursor to stage start/end* provenance |

≈ 24 B per normal encoded stage; overflow records pay for exceptional timestamp
ranges without truncating or repairing them. A decoded implementation may use a
wider 32 B layout if that is faster, but only decoded LRU pages pay that cost. The
reverse mapping (event tx id → stage locator), needed only when an
event-table row is activated, is a lazily built sorted permutation over `event_tx`,
binary-searched — no resident hash map. An optional compression step (worth doing only
if profiling demands it) stores `event_tx` as a delta from the instruction's `tx_id`,
with a side table for overflow; the architecture treats that as an internal layout
choice invisible above the accessor layer. Stage-level highlight is per-tile runtime
state `(row, stage locator)`, distinct from the shared focused transaction.

Within one instruction, stages are sorted by `(start, lane, input order)` at build
time for deterministic rendering and a short containment scan; pathological long
spans may add the prefix-max-end accelerator described in §7.2.

**Annotation store.** Per-stage attributes ("`X: d:0x7 = fu(a:0x0, b:0x7)`", cache
hit/miss notes) are typed triples `{name: u16, tag: u8, value: u64-or-dict-ref}` in a
flat array sliced by `attr_off` — ~12–16 B each, only paid where annotations exist.
Tooltips and search *format* these on demand; nothing pre-renders strings. Attribute
types outside the compact set remain typed overflow values; the builder never calls a
general display formatter in its hot loop. Unknown or mistyped convention fields stay
inspectable and add a quality diagnostic rather than being coerced.

### 5.3 Instruction-level memory budget

The important split is **resident navigation data** versus **encoded or decoded
detail**. Quoting one all-resident total hides the property that makes eviction work:

| Component | Design budget | Residency |
|---|---|---|
| Row directory (§5.1) | ≤ 64 B/instruction | Always resident; ~64 MiB at 1 M rows, ~640 MiB at 10 M |
| Lookup/layout/aggregate summaries (§5.4, §5.6–§5.7) | Measured separately; absent columns/index variants are omitted | Resident and page-granular |
| Normal stage detail (§5.2) | ≤ 24 B/stage before attributes | Encoded backing pages; decoded pages live in a 256 MiB native / 64 MiB web LRU initially |
| Dependency detail (§5.5) | ~28 B/edge plus compact CSR/permutation indexes | Encoded/paged; only visible/focused adjacency is decoded |
| Render + density results | Output-sized | Separate 64 MiB native / 24 MiB web initial budget |
| Unique text | O(unique text) | Shared dictionary fragments; never copied per record |

The generic-store row uses the measured struct sizes of the current parser fork
(`Transaction` 112 B, `Attribute` 48 B, `TxRelation` 48 B + two hash-map index entries,
plus `EventIndex`'s duplicated per-tx lookups), and a representative ten-stage row is
roughly kilobytes rather than tens of bytes. That loaded-container path is acceptable
for the functional milestone, but it is not the multi-million-row architecture.
Scale claims require the paged query path (§6.4), forced-eviction measurements, and
peak/resident-memory reports; they are not derived from the table above.

### 5.4 Point indexes

* **Time → candidate rows.** Recorded begin order defines identity even when timestamps
  move backward; rows are never reordered. Every row page records min-begin and
  max-end, and a segment tree over those summaries prunes time-window queries without
  assuming monotonic timestamps. Fully monotone traces additionally get a compact
  fast-path binary-search index. Regressions set quality bits, not new semantics.
* **RID → row.** Per-thread dense RIDs use a direct vector plus duplicate/missing
  bitsets; sparse RIDs use sorted `(rid, row)` pairs. Duplicate or missing RIDs mark
  only those keys ambiguous, which the UI reports for RID-addressed commands or sync
  (UX §9.3, §13.1).
* **SID → row** uses the same density test, unpartitioned, and retains all duplicate
  candidates.
* **tx_id → row and event id → stage.** Dense ids use direct ordinal vectors; sparse
  ids use sorted `(id, locator)` pairs with binary search. Representation choice is
  recorded in diagnostics so memory reports are explainable. Hashing is reserved for
  bounded builder-tail joins, never one permanent heap entry per final record.

### 5.5 Dependency graph

Relations between instruction transactions (excluding the `parent_of` links whose sink
is an event — the discriminator already implemented in
`transaction_events.rs`) become edges:

| Field | Type |
|---|---|
| `prod_row`, `cons_row` | u32 × 2 |
| `rel_name` | u16 (relation-name table; drives per-relation color legend) |
| `flags` | u16 — endpoint kind/fallback bits (hollow endpoints, UX §8.1) |
| `prod_tick`, `cons_tick` | u64 × 2 — tier-1 anchors, sentinel when absent |

≈ 28 B/edge. Anchor resolution implements UX §8.1's three-tier chain:

* **Tier 1 — relation endpoint timestamp.** A relation endpoint may be a stage
  *event* transaction rather than the instruction itself; such endpoints resolve to
  the event's parent row (via the event index) and record the event's own timestamp
  into `prod_tick`/`cons_tick` at build time. Instruction endpoints leave the
  sentinel.
* **Tiers 2–3 — resolved at draw/walk time**, not build time: first stage whose name
  is in the configured execution-stage set, else the transaction start, with the
  hollow-endpoint flag. Draw-time resolution touches only visible edges (O(stages per
  row) each), and dependency walking touches one row's edges — so the
  execution-stage set can live in per-tile options (global default in config) without
  any shared-model invalidation problem when the user edits it.

Storage is logically two CSR adjacencies (by producer row and consumer row) for
focus-driven emphasis and `Alt+←/→` dependency walking (O(degree)), emitted into
detail pages. A compact draw index buckets edges by maximum endpoint page and span
class. View extraction visits only buckets intersecting the visible row window plus a
small prefetch margin; very long edges are indexed by endpoint page rather than kept
in an always-scanned list. Buckets seal appendably during progressive loading because
maximum endpoint is normally the newly seen consumer.

Visible-arrow geometry has a configurable hard primitive budget and stable recorded-
order truncation/aggregation, with a suppressed-edge count in the overlay. Thus an
adversarial high-degree row cannot turn one frame into O(trace). Dependency walking
still sees the full adjacency off-thread; focused direct-neighbor drawing uses the
same visible budget.

### 5.6 Range aggregates — the zoomed-out machinery

Three small structures make every zoomed-out regime O(screen):

1. **Time-envelope RMQ.** Blocked min/max tables over `begin`/`end` (block 64,
   adapting the `SignalRMQ` design in `analog_signal_cache.rs`): O(1)
   `[min_begin(rows a..b), max_end(rows a..b))` without assuming timestamp
   monotonicity.
2. **Prefix sums** over flushed/committed/unknown flags (u32 per 64-row block +
   bit-rank inside): O(1) counts for any row range — density tinting, status-strip
   totals, and region statistics.
3. **Skyline pyramid.** For block sizes 64·4ᵏ: `{min_begin, max_end, count,
   flushed_count, occupancy[32]: u8}` where `occupancy` is a 32-bucket saturating
   histogram of stage coverage across the block's time extent. ~2 B/row total across
   all levels. Level k+1 folds level k.

Rendering below one pixel per row picks, **per pixel row**, the level with ≥ 1 block
inside that pixel row's *physical* span (the span can vary hugely across pixel rows
when hiding is active — a flush storm packs thousands of physical rows into one
visible pixel row) and resamples occupancy strips into screen columns — dense
diagonals, stall gashes, and flush wedges emerge exactly as UX §5.2 describes, at
O(pixels · 32) per frame. Exact range edges combine complete level-0 blocks with a
≤ 64-row scan at each side. The minimap (§7.8) and the "aggregated pixel"
tooltip/zoom-in interaction read the same pyramid.

**Hide-flushed interaction.** With hiding active, a pixel row covers a *visible*-row
range whose physical rows interleave with hidden flushed rows; aggregates computed
over the enclosing physical range would wrongly include the hidden rows (a hidden
flush storm must actually disappear from the skyline). The RMQ and pyramid therefore
exist in two variants: **all-rows** (default) and **non-flushed**, where only rows
explicitly marked `flushed == true` contribute neutral elements. Unknown flush state
remains visible rather than being guessed committed or flushed. Both variants share
block geometry; the non-flushed variant is built lazily off-thread the first time any
tile enables hiding, then cached on the model. Prefix sums need no second copy.

### 5.7 Visibility index (hide flushed ops)

Hiding is model-wide data (flushed is a fact of the trace), so the model owns one
visibility structure, shared by tiles that enable the toggle: a bitvector with 512-bit
blocks and cumulative u32 ranks — `rank(row) → visible index` in O(1),
`select(visible index) → row` in O(log blocks) (binary search on block ranks +
popcount within the block). ~0.2 B/row.

Everything row-visible operates in visible space through this index: rendering,
diagonal follow, anchor alignment, PageUp/Down, sync anchoring. Find reports
hidden-hit status by checking the bit (UX §9.2). Toggling costs nothing but a draw
cache invalidation; the top-of-screen anchor is preserved by converting the anchor row
through rank/select across the toggle.

### 5.8 Page-query and cache contract

Model consumers do not hold raw page locks. An immutable snapshot offers row columns
and summaries directly, plus non-blocking detail queries with three outcomes:
`Ready(page)`, `Pending`, or `Unavailable(diagnostic)`. A miss schedules prioritized
prefetch for the visible/focused window and returns immediately. The renderer uses row
extents or summary density until detail arrives; search/statistics workers may await a
page off the UI thread while continuing independent pages.

Decoded detail pages use a byte-budget LRU. Visible and focused pages are pinned only
for the frame/job snapshot that uses them. Text-layout, detailed-render, density,
search, and statistics caches have independent keys and budgets (§10.7), so a wide
search cannot evict the visible render working set by accident.

Derived local pages may be cached in the OS cache directory under a source
fingerprint, source generation, parser version, and convention version. That cache is
an optimization only: writes are atomic, deletion cannot change semantics, and no
sidecar is written beside the trace without a separate product decision.

---

## 6. Ingest: building the model

### 6.1 Existing parser baseline and query boundary

The `ftr-parser` fork has fixed the scalability blockers documented in FTR_EVENTS.md
(see `ftr-parser/PERFORMANCE.md`): u64 timestamps, `Arc<str>` interning against the
on-disk dictionary, relations stored once with prebuilt source/sink index maps
(attachment is O(1) per transaction), and lazy per-stream body loading — file-backed
loads parse only headers, dictionary, directory, and relation chunks, recording
`tx_block_ids` byte offsets per stream for later loading.

Discovery of conforming generator pairs is also already implemented:
`EventIndex::pair_generators` (`libsurfer/src/transaction_events.rs`) matches
`<base>.events` siblings per stream — the Konata entry points (§10.4) reuse it
verbatim, including the rule that the pair is sufficient before relations are checked
(UX §3).

Those are useful primitives, not yet the scale interface. `TransactionContainer`
adds a parser-neutral query facade for:

* stream/generator metadata and dictionary fragments;
* block metadata and byte/record progress;
* sequential transactions for selected generators in recorded order;
* sequential relations in recorded order;
* typed attributes that retain dictionary ids; and
* point lookup by compact transaction/event locator.

The first adapter wraps today's in-memory `FTR`, enabling the functional tile without
duplicating convention logic. The scale adapter decodes FTR blocks on demand. Given
the same trace, both adapters feed the same normalizer and must produce equivalent
canonical model pages and quality diagnostics.

The FTR directory must retain one `BlockMeta` per transaction/relation chunk: encoded
offset/length, compressed state, uncompressed length, stream id, recorded ordinal,
declared time bounds where available, and parse/validation status. This makes progress
honest and enables prioritization, random access, and cache keys. The present directory
does not retain all of that metadata, so it is a parser milestone rather than an
assumed capability.

### 6.2 Build algorithm (from a loaded stream)

A single structural pass, linear in transactions + relations:

1. **Instructions.** Iterate the parent generator's transaction vector in recorded
   begin order (this defines row/fetch identity; never sort by timestamp): fill
   instruction columns; decode and type-check the well-known attributes
   (`label`, `detail`, `insn_id_in_sim`, `thread_id`, `retire_id`, `flushed`) into
   columns; intern strings into the model dictionary; record `tx_id → row`; update the
   row-page time summaries and warning flags.
2. **Stages (two-pass CSR).** Iterate the `.events` generator once, resolving each
   event's parent through the relation sink index — pass one counts stages per row to
   size `stage_off`, pass two fills records and annotation triples. Orphans, multi-
   parent events, and out-of-range events take the quality paths of §12 rather than
   being dropped. Per-row sort of (few) stages afterwards.
3. **Dependencies.** Filter `FTR::tx_relations` for edges whose endpoints resolve to
   instruction rows — directly via `tx_id → row`, or through the event's parent when
   an endpoint is a stage-event transaction (which also supplies the tier-1 anchor,
   §5.5); sort the draw list; build both CSRs with counting sort. O(E log E) worst
   case, E ≈ instructions.
4. **Aggregates.** RMQ, prefix sums, pyramid, visibility index — all linear scans.

The pass runs off the UI thread via `perform_work` (`libsurfer/src/async_util.rs`),
reporting `Message::KonataModelProgress` per chunk and `Message::KonataModelBuilt` at
the end, guarded by revision + generation exactly like `Message::TableCacheBuilt`
(`table_controller.rs`). On wasm, a Web Worker receives owned transferable buffers
when available; the fallback runs the same chunk loop cooperatively in bounded slices
(~5 ms) and yields before the next frame. No algorithm relies on shared-memory wasm
threads.

**Thread-safety prerequisite (small, ours to make):** background builders need a
stable snapshot of the loaded stream while the UI keeps running. The parser fork will
hold transaction bodies behind `Arc` at generator granularity (`transactions:
Arc<Vec<Transaction>>`), so a builder clones two `Arc`s and never blocks the UI;
stream (re)loads swap the `Arc` and bump the source generation, which orphans stale
builds via the revision guard. This mirrors how table models snapshot their inputs.

### 6.3 Progressive loading

Today the FTR path is fully synchronous: `parse_ftr` runs inline on the caller
(`wave_source.rs::load_transactions_from_file`) and stream bodies load on the UI
thread at display time. Header parsing is fast (bodies are skipped), so the gap is
body loading and model building — both become asynchronous and incremental:

* The parser gains a **block visitor** API: iterate a stream's `tx_block_ids`,
  decompress + decode one block at a time, and hand each decoded batch to a callback
  with progress (bytes consumed / total). This is a natural refactor of the existing
  `load_transactions` loop, which already processes blocks one at a time.
* The Konata build consumes batches as they arrive. The append story is spelled out
  per structure, because it is where naive designs die:
  * Instruction columns, time summaries, and adaptive id indexes grow at the row-page
    tail. RMQ/pyramid blocks seal with that page; the small cross-page tree is
    appendable and is copied into each published snapshot — amortized O(1)/row.
  * **Stage CSR is per detail page**, with a small per-page *late-arrival overflow
    list* (sorted, merged at query time) for stage events that arrive after their
    instruction's page sealed. Events whose parent has not been seen yet wait in a
    bounded pending run keyed by parent tx id. If it exceeds its memory budget it
    spills to the derived cache; only events unresolved at end-of-load count as
    orphans, so the quality badge is not polluted by stream interleaving.
  * Dependency draw buckets append by maximum endpoint page (§5.5). Final CSR pages
    replace recent edge runs as they seal; during load, focus queries may scan only
    the bounded active-tail run.
  Relations were parsed eagerly at open, so parent and dependency attachment per
  batch is index lookups, with the pending pool absorbing arrival-order skew.
* After each batch: progress message → tile repaints with rows so far, totals marked
  provisional (UX §5.1). Cancellation: source close/reload bumps the generation and
  the build loop observes a cancel token per chunk, like table cache builds.

Reload preserves anchors by identity-then-timestamp (§4.3); append-style reloads (a
growing trace) reuse immutable pages only when the source fingerprint and block
directory prove their encoded bytes unchanged. Otherwise a new generation builds
beside the old snapshot; the tile keeps the old image with a stale/reloading badge
until the first useful new snapshot is ready. Restoration tries exact transaction id,
then unique `(thread id, RID)`, then unique SID, then saved timestamp/nearest recorded
row. A partial parse keeps all consistent published pages plus a fatal diagnostic.

### 6.4 Streaming build for very large traces

At 10 M instructions the generic representation cannot be resident. The block visitor
enables a second consumer: build the
KonataModel **directly from decoded blocks without retaining `Vec<Transaction>`** —
decode a block, feed the builder, drop the block. The container keeps the stream
marked unloaded for its own purposes; only the compact row directory, summaries, and
budgeted detail working set are resident. Until waveform/table consumers migrate to
the query facade/model-backed path (§10.5), opening a legacy whole-stream view at this
tier must require an explicit capability/cost decision or report that it is
unavailable; it must not silently defeat the memory bound.

The bounded-memory builder has three logical passes. Small in-memory traces may fuse
them, but must produce the same order and diagnostics:

1. **Discover and publish parent rows.** Scan the selected parent/event generators in
   recorded order, assigning a row only when a parent first appears. Publish complete
   row pages immediately; write stage candidates to temporary page runs keyed by event
   id.
2. **Resolve relations.** Stream relation chunks in recorded order. Join
   `parent_of` event links and instruction dependencies to row/event locators. Large
   joins use sorted runs and merge joins; only the bounded active tail may use a hash
   table.
3. **Canonicalize detail.** Group stages and edges by row, stable-sort each short stage
   span, emit CSR detail pages, compute quality/overview summaries, and atomically
   replace provisional detail. Rows may therefore appear before stages, but never
   change identity.

One genuine scaling risk sits outside the streams: **relation chunks parse eagerly at
file open** into `TxRelation` (48 B, `ftr-parser/src/types.rs`) plus two
`HashMap<TransactionId, Vec<usize>>` index entries declared on `FTR` in the same file
(~90–100 B combined per relation, populated in `ftr_parser.rs`). At 10 M instructions
× ~11 relations (10 stage links + wakeups) that is ~110 M relations ≈ **15–16 GB at
file open**, before any view exists — eager relations, not stream bodies, are the
hard wall at this tier. The follow-up in the parser fork: store relations columnar
(name id + four u64s ≈ 36 B), replace the per-id Vec maps with sorted-by-sink and
sorted-by-source permutation arrays (8 B each, built once, binary-searched), and let
the Konata builder consume-and-release `parent_of`-to-event relations (the dominant
class, ~10/11 of all relations) as it attaches stages. That leaves ~50–60 B for each
*retained* relation, removes the hash-map allocation storm, and drops open-time
relation residency to roughly the wakeup edges alone (~0.5–1 GB). This is required
for the multi-million tier and is listed as its own milestone. The specific byte
estimates are hypotheses to validate with the performance harness, not release claims.

---

## 7. Rendering

### 7.1 Two-tier pipeline

Copied from the main canvas (`drawing_canvas.rs`): a **draw-command generation** step
producing a `KonataDrawData` cache, and an **immediate-mode painting** step running
every frame from the cache.

* Cache key: (viewport snapshot, canvas rect, model generation + loaded-row count,
  visible-page revisions, view options, theme epoch, focus/emphasis state). Regenerate on mismatch — same
  discipline as `CachedDrawData` + `invalidate_draw_commands`, held per tile in
  `KonataRuntimeState`.
* Generation cost is bounded by §7.2–§7.6 to O(screen); on a 4K canvas it is
  comfortably per-frame even during animated pans, matching how the waveform canvas
  regenerates while animating.
* Painting emits: one batched mesh for all stage rectangles (per-vertex colors give
  the vertical gradient for free), one for overlays/strokes, bounded `painter.text`
  calls, arrow paths, ruler/cursor/marker lines. No textures except the minimap
  (§7.8); everything stays CPU-tessellated egui shapes, which is what the
  snapshot-test renderer consumes (§13).
* Before generating detail, the frame queries visible pages plus a small prefetch
  margin. Ready pages contribute exact stages; missing pages schedule non-blocking
  decode and contribute row extents or page-summary strips. The frame never waits for
  I/O, decompression, or worker geometry.

**Hit-testing is arithmetic, not widgets.** The waveform canvas allocates an egui
widget per visible transaction (`allocate_rect` per box) — acceptable at hundreds,
wrong at tens of thousands. The Konata canvas allocates **one** painter response;
pointer → `(tick, visible row)` by inverse viewport math; row → stage by binary
search in the row's stage slice; overlap resolution returns all hits ordered by
(lane, start, index) for click-cycling and the multi-hit tooltip (UX §5.1). Same
approach for label-pane rows, ruler, minimap lens.

### 7.2 Culling

* Rows: `top_visible_row .. top_visible_row + height/row_height` in visible space —
  O(1); translate through `select` when hiding is active.
* Stages within a row: when its detail page is ready, typical short slices (~10–40)
  use a linear start/end window scan. Longer slices carry a prefix-maximum-end array:
  binary-search the last possible start, then scan backward only while the prefix max
  can still cross the left edge. This keeps long early stalls correct even though end
  times are not monotone. Hit-testing (§7.1) uses the same interval query. A missing
  page yields a row-level hit plus a prefetch request, never a synchronous decode.
* Dependency arrows: §5.5 extraction; skipped entirely below the arrow LOD threshold.

### 7.3 Level-of-detail regimes

Effective detail = min(row height, one-cycle pixel width) per UX §5.2; thresholds
are user-configurable. A predicted primitive count above the frame budget forces the
next coarser regime even when geometric thresholds would allow more detail, so a
malformed high-stage-count row cannot exhaust a frame. The density regime is
O(screen); detailed regimes are bounded by visible interval candidates and the hard
primitive budget:

| Regime | Draw algorithm |
|---|---|
| **Full detail** (≥ ~10 px) | ≤ ~100 visible rows × visible stages: gradient rect + border per stage; stage-name galley in the first cycle cell, trailing cells numbered `1 2 3…` — text laid out only after a width check, truncated to available pixels first (the `draw_region` fits-then-layout trick from the waveform canvas keeps galley count bounded by what is actually readable). Zero-duration stages draw diamonds; overlapping same-lane stages inset by lane order. |
| **Boxes** (~4–10 px) | Same geometry, no text, no per-cell numbering. |
| **Strips** (~1–4 px) | Rects only, no borders; within a row, consecutive stages that land in the same pixel column merge into one run (pixel-advance scan, the same coalescing rule the transaction renderer applies). Arrows hidden. |
| **Density** (< ~1 px/row) | Per screen pixel row: map to row range, then O(1) envelope `[min_begin(a..b), max_end(a..b))` + O(1) flushed count → one tinted envelope bar. When a pixel row covers > 64 rows, switch to pyramid blocks and resample their 32-bucket occupancy strips into screen columns for intra-envelope texture. Tooltip shows the aggregated instruction/time range and count; click zooms into it. |

Flushed overlays, warning outlines, alternating stripes, and off-trace shading are
per-visible-row decorations added in the same passes.

**Lanes and uniform row height.** Every O(1) mapping above (row culling, arithmetic
hit-testing, density pixel-row ranges, diagonal anchors) relies on uniform row
height. Split-lane *natural height* therefore multiplies the row height by the
**model's global lane count** (the lane table's size — matching original Konata),
never by per-instruction lane counts; *fixed op height* subdivides the unchanged row.
Lane modes thus only alter the row→y and lane→inset transforms, and no
variable-height row index is ever needed.

### 7.4 Label pane

A virtualized list aligned to canvas rows (visible-space indices), rendered with the
same truncate-before-layout rule; hidden below the readable threshold. It shares the
canvas's row transform so splitter drags and scroll are exact. Accessibility metadata
(UX §13.2) attaches here, where egui's widget model fits naturally — one widget per
*visible* label row is bounded and cheap.

### 7.5 Arrows

Straight inside-lines from `(prod_tick, prod_row)` to `(cons_tick, cons_row)`; left
curves as cubic beziers pinned to the two rows' begin positions at the pane's left
edge (both clipped to the canvas rect). Focus emphasis: with a focused row, its
in/out CSR slices render highlighted while the ambient draw list renders dimmed.
Producer-chain highlight = reverse BFS over the producer CSR into a row bitset,
bounded by visited count and run asynchronously with the standard cancel pattern if
it exceeds a frame budget. Hollow endpoints render per the fallback flags.

### 7.6 Cursor, markers, ruler

Cursor and markers live in `WaveData` (`cursor: Option<BigInt>`, `markers:
HashMap<u8, BigInt>`) — global, serialized, already shared by tables via
`Message::CursorSet`. The Konata canvas draws them as vertical pins at
`tick = clamp(BigInt→u64)`, moves the cursor on ruler clicks by emitting the same
message, and never stores its own copy — cross-view sync (UX §10.1) is therefore free.
The ruler picks cycle/time labels with the standard adaptive tick-density rule
(target label spacing in px → round step to 1/2/5×10ᵏ cycles or the existing time-unit
formatting in trace-time mode); marker pins stack when coincident.

### 7.7 Overlay comparison and scroll sync

* **Sync groups**: a serialized group id per tile plus shared group state (zoom,
  splitter position, alignment mode). A scroll in one tile emits viewport messages
  for the group; messages carry the origin tile id and a monotonically increasing
  group sequence, and only user-originated changes rebroadcast. This removes feedback
  loops and stale updates. The **alignment mode** is an explicit
  enum — `(thread id, retire id)` via the RID index, stable fetch ID (row index
  directly), or timestamp (via the time index) — chosen by the user when RIDs are
  absent or non-unique; Surfer never falls back silently. The anchor is **re-resolved
  on each user-initiated scroll** (the committed instruction nearest the origin
  tile's viewport center whose key resolves in every synced model), so traces with
  different flush/stall distributions cannot drift apart; each follower translates
  the anchor through its own indexes and preserves its relative fetch-tick offset.
  Duplicate keys are detected as adjacent equal entries while the sorted RID index
  builds; unmatched/duplicate counts report in the tile header, and an ambiguous key
  degrades only lookups of that key, not the whole sync.
* **Overlay tiles** serialize a spec with a *list* of sources (primary + overlays,
  each with its flat-color assignment and the alignment mode), so `.surf.ron`
  restores the combined tile; the runtime holds one model `Arc` per source plus a
  per-source **affine transform** — `(row offset, tick offset, optional tick scale)`
  where the scale is the clock-period ratio when both traces carry clock mappings —
  derived from the current anchor. Two runs' absolute timestamps are never assumed
  comparable. An anchor resolvable in only one source renders that source unshifted
  and reports the mismatch in the tile header rather than guessing. Back trace
  renders first, front at ~50 % alpha (vertex alpha in the same mesh path);
  pointer-hold emphasis just reorders/boosts alpha for one source in the cached
  command set.

### 7.8 Minimap

Rendered from the pyramid into a small `ColorImage`/texture (one column ≈ the whole
time range, rows compressed), rebuilt asynchronously on model growth, theme change, or
hide-toggle — never per frame. The lens is viewport state drawn on top; drags map
linearly to visible-row space. Cost: O(minimap pixels), texture size ~(width ≈ 16–24
px × canvas height).

### 7.9 Frame budget (worst cases)

| Regime | Primitive bound | Estimate at 1440p |
|---|---|---|
| Full detail | rows × visible stages ≤ ~100 × ~40 | ~4 k rects + ≤ ~1 k galleys (cached by egui) |
| Boxes/strips | ≤ ~1200 rows × min(stages, px-runs) | ≤ ~20–25 k rects → ~100 k vertices, one mesh |
| Density | 1 query + ≤ 32 strip samples per pixel row | ~1.4 k queries + ~45 k samples |
| Arrows | indexed visible candidates, hard-capped | configurable; start at ≤ 4 k paths with suppressed count |

All well inside egui's tessellation headroom; the `performance_plot` feature's frame
instrumentation (existing named regions + 60/30 fps reference lines in
`benchmark.rs`) gets two new regions — Konata command generation and Konata painting —
so regressions are visible in-app.

### 7.10 Accessibility and keyboard focus

Widget-free hit-testing (§7.1) removes the *implicit* accessibility egui widgets
provide, so the canvas supplies it explicitly, still bounded by the screen:

* The tile keeps an explicit **keyboard cursor** — current row, optionally current
  stage within it — in runtime state. `Space` focuses it (defaulting to the
  view-center row), arrow keys move it, and Tab traverses tile controls → ruler →
  label list → canvas as regions (UX §6.3/§13.2). This cursor is also what
  `Shift+Space` pins tooltips against.
* Under the `accesskit` feature, the canvas synthesizes a **virtual node subtree for
  visible content only** — one node per visible label row (ID, SID/TID/RID, label,
  flushed state, stage count) and per visible stage of the keyboard-current row
  (name, start, end, duration, lane, parent, warning state), regenerated with the
  draw-command cache. Node count is bounded by the same O(screen) budget as
  drawing; off-screen rows are reachable by moving the keyboard cursor, which
  scrolls the viewport and refreshes the subtree.
* Focus indication is drawn as outline + glyph, never color alone, and all context
  menus/popovers are ordinary egui widgets that inherit keyboard operation.

---

## 8. Interaction algorithms

* **Diagonal follow (UX §6.1).** On a vertical scroll of Δ visible rows with center
  anchor `v`: horizontal compensation `= begin[row(v+Δ)] − begin[row(v)]` (through
  `select` in visible space; actual timestamps, so bubbles and multi-issue fronts
  don't cause drift). Zero at the ends or when the neighborhood shares one timestamp.
  Shift suppresses compensation. Fractional rows interpolate between neighbors so
  smooth (pixel-level) wheel deltas don't stair-step.
* **Zoom at pointer.** Solve `left_tick`/`top_visible_row` so the pointer's
  (tick, visible row) is invariant under the new scales; animate current → target.
* **Label-click align / Adjust position.** Set `left_tick = begin[row]` (animated);
  Adjust position uses the top visible row, or the sync group's `(tid, rid)` anchor
  when synced.
* **Jumps** (`konata_goto_row/rid/sid/cycle`): resolve through §5.4 indexes; RID
  ambiguity prompts for thread as specified. A jump or bookmark whose target row is
  hidden by *Hide flushed ops* scrolls to the nearest visible position (`rank` of the
  target) and shows the same "target is hidden" notice find uses — never a silent
  landing on the wrong row.
* **Esc priority stack.** Each tile keeps an explicit stack of transient UI (context
  menu, popover, active search, result card, pinned tooltip, focus) — one pop per
  press, deterministic (UX §6.3).
* **Bookmarks** store `(tx_id, tick, zoom)`; resolution by `tx_id → row`, fallback to
  the timestamp via the time index (UX §6.4). Slots are **per source generator**, not
  per tile (UX: "ten numbered slots per source"), so they live in a
  `UserState`-level map keyed by the model's stable spec, shared by all tiles over
  that generator.
* **Tooltips** (UX §7.1: hover-into, selectable, copyable, pinnable) are not stock
  egui tooltips — each tile renders at most one custom popup area whose hover region
  is the union of the source cell and the popup rect, with selectable text and a
  copy affordance; pinning moves it onto the Esc stack.

---

## 9. Search

Scope per UX §7.3: identity numbers, label, detail, stage names, and stage
annotations. The model exposes these as typed fragments; it never persists one joined
`String` per row. Search reads immutable model pages through §5.8 and allocates no
per-row result object:

1. Compile the regex before replacing the last valid result (invalid pattern → inline
   error while keeping the prior result).
2. Build a circular row-page plan beginning after the current row for forward search
   or before it for reverse search. Each worker reuses one scratch buffer to join a
   row's typed fragments with stable separators; regex literal prefilters apply
   normally.
3. Store hits in a row bitset plus per-page hit counts. Next/previous is rank/select
   over those structures, not a worst-case vector containing every matching row.
4. Publish page completions and progress in plan order. Native workers may scan pages
   in parallel, but the first hit is announced only after every earlier plan page has
   completed, so scheduling cannot change which match is “next.”
5. Cancellation and staleness: cancel token + revision, the table-cache pattern.
   `F3`/`Shift+F3` reuse the compiled program from the current position. Hidden hits
   are detected via the flushed bit + hide state and reported, not skipped silently.
6. Native uses `perform_work`; wasm uses a Web Worker when available and otherwise
   processes time-budgeted page slices.
7. **to table**: emit the pattern as a `TableSearchSpec` display filter on an
   instruction-table spec — the table subsystem's async filter machinery does the
   rest. Note the scope subtlety: the *existing* transaction table's searchable text
   does not include per-stage annotations, so its results would differ from the find
   bar's. The model-backed table (§10.5) exposes the same composed row text as
   `search_text` and is therefore pulled forward to whenever find-to-table ships, so
   both searches always agree.

The first implementation remains a progressive full scan. A trigram or other index is
added only if benchmarked search latency justifies its resident memory, and then only
as a candidate prefilter that cannot change regex semantics. G4 remains an acceptance
target until the measured harness records it.

---

## 10. Integration with Surfer

### 10.1 Tile

New pane variant `SurferPane::Konata(KonataTileId)` beside `Table(TableTileId)`
(`libsurfer/src/tiles.rs`), with an id counter on `SurferTileTree` mirroring
`next_table_id`. Split/tab/close/serialize behavior comes from `egui_tiles` + the
existing `Behavior` impl; `pane_ui` gains one match arm dispatching to the Konata
tile renderer; closing follows the `on_tab_close` → deferred-removal path that tables
use. The `mem::take`/restore borrow dance in `draw_tiles` extends to the Konata tile
map. Closing drops runtime handles, decrements the shared projection consumer count,
and cancels a build only when no consumer remains. Tile title:
`Konata — <generator> (<file>)`.

### 10.2 State split and async protocol (the table template)

| Piece | Lives in | Contents |
|---|---|---|
| `KonataTileState` | `UserState.konata_tiles: HashMap<KonataTileId, _>` (serialized) | `spec` (source id + generator ref — with `references_source`/`remap_sources` hooks like `TableModelSpec`, so source close/remap and deferred state restore work), `config` (color scheme, lane mode, hide-flushed, arrow style, LOD thresholds, ruler unit, splitter, sync group, clock override, execution-stage set), viewport. Bookmarks live in a sibling per-source-generator map (§8) |
| `KonataRuntimeState` | `SystemState.konata_runtime` (never serialized) | `model: Option<Arc<KonataModelEntry>>`, draw cache, revision, cancel token, find state, Esc stack, hover cache |
| `KonataModelEntry` | `SystemState.konata_models: HashMap<KonataModelKey, Arc<_>>` | the latest published model *snapshot* (§5) behind a swappable holder — a mutex-guarded `Arc` slot, not `OnceLock`, because progressive load publishes a new snapshot per batch — keyed by the full source/generator/events/generation/convention pipeline key; deduplicated across tiles exactly like `table_inflight` deduplicates cache builds |

Message flow, copied from `BuildTableCache`/`TableCacheBuilt` and
`BuildAnalogCache`/`AnalogCacheBuilt`: tile draw detects missing/stale model → pushes
`Message::BuildKonataModel` → controller (new `konata_controller.rs`) cancels prior
build, bumps revision, spawns via `perform_work`, tracks `OUTSTANDING_TRANSACTIONS`
so the reactive repaint loop stays awake → progress/built messages carry the revision
and are dropped when stale. `SystemState` gains `konata_caches_ready()` beside
`table_caches_ready()` for the snapshot-test wait loop. All Konata messages group
into one `lib.rs` match arm forwarding to the controller, as table messages do.
New serialized fields use serde defaults so older state files remain loadable.

### 10.3 Cross-view coupling

Exclusively through existing shared state, no new infrastructure:

* **Focused transaction**: read/write `WaveData.focused_transaction` via
  `Message::FocusTransactionFromSource` — tables already focus transactions this way
  (`TableAction::FocusTransaction`), and the Konata controller reacts to focus
  changes by scrolling/highlighting when the tx maps into a model (`tx_id → row`). An
  instruction focuses its parent transaction; a stage focuses the event transaction,
  which the model maps back to its parent row. The originating tile suppresses
  auto-scroll for its own focus message while other tiles navigate to the identity.
  The current message/state also carries an optional cloned `Transaction`; the paged
  path supplies the source-qualified identity without forcing that clone. Moving the
  shared focus to identity-only later is useful cleanup, not a tile prerequisite.
* **Cursor/markers**: §7.6, via `Message::CursorSet` / marker messages.
* **Show in event/transaction table**: emit the existing
  `Message::OpenTransactionTable`-family messages with pre-set filters; **Show in
  waveform view** = focus + cursor-set, which the waveform already honors.

### 10.4 Entry points, commands, WCP

* Sidebar context menu and auto-suggestion use `EventIndex` pairing to enable
  **Open in Konata view**; menu and status-bar hint are plain messages.
* Command palette: register `konata_view_new`, `konata_goto_row/rid/sid/cycle`,
  `konata_find`, zoom, and bookmark verbs in `command_parser.rs::get_parser`
  (string list + parser arm each, with fuzzy generator-name suggestions like
  `transaction_table`); history comes with the prompt.
* WCP: new variants on `WcpCommand` (`surfer-wcp/src/proto.rs`), match arms in
  `wcp_handler.rs`, plus the manually maintained greeting list (which already drifts
  from the enum — extend it deliberately).

### 10.5 Tables over the model (find-to-table at scale)

Phase 1 reuses the existing per-generator transaction/event table models. As a
follow-up, a `TableModelSpec` variant backed by `Arc<KonataModel>` implements
`TableModel` directly over the columns (row count, cells, sort keys, and lazy search
fragments),
making instruction tables on multi-million-row traces cheap and giving find-to-table
and statistics results a container-independent path — relevant precisely when §6.4
skipped materializing the container. It uses `SearchTextMode::LazyProbe` over the same
typed fragments as §9, so table filters and the Konata find bar cannot disagree about
child-stage annotations.

### 10.6 Local, byte-backed, wasm, remote, reload, and theming

* **Local files** use the block directory for seekable cancellable reads; the OS page
  cache and decoded-detail LRU provide locality.
* **Byte-backed files/raw URLs** use a cursor over shared immutable encoded bytes and
  the same block directory, rather than eagerly expanding all transactions. A legacy
  raw URL may still require full download before parsing; that is an explicit fallback,
  not the scaled-remote path.
* **Wasm** uses owned transferable page buffers: Web Workers for build/analysis when
  available, cooperative time-sliced fallback otherwise. No design depends on wasm
  shared-memory threads.
* **Surver** currently exposes waveform hierarchy/timetable/signal payloads, not
  transaction pages. Scaled remote Konata therefore requires versioned advertised
  capabilities and endpoints for pipeline descriptors/clock metadata, projection
  revision/progress/quality, row-directory ranges, compressed detail pages,
  dictionary fragments, and optional overview summaries. Requests carry source
  revision and page ids; responses are independently cacheable and stale revisions
  are rejected. If the capability is absent, the entry point is disabled with the
  UX-specified explanation unless the user explicitly selects the legacy full-download
  fallback; it never silently downloads an unbounded object graph.
* **Reload / file watch**: existing reload machinery bumps the source generation;
  §6.3 handles anchor preservation and append detection.
* **Theming**: Konata colors become theme fields with defaults in
  `default_config.toml`/`themes/*` (stage palette generation parameters, stall gray,
  flush overlay, warning tint, ruler/minimap chrome). Scheme algorithms: *Auto* =
  hue by stage-name first-appearance index; *Unique* = stable hash of the name;
  *ThreadID* = hue by tid, lightness by stage depth; flats; user schemes from config
  with `auto` wildcards. All schemes route through a contrast-clamp against the
  active theme background, and a CVD-safe palette ships as a builtin (UX §9.1).
  Palette resolution is cached by integer ids and theme/options revision. Focus,
  flush, and warning are separate outline/overlay layers rather than color
  substitutions, so their meaning survives flat and high-contrast schemes.

### 10.7 Cache and concurrency policy

| Cache | Key | Eviction/policy |
|---|---|---|
| Shared projection | full pipeline key (§5) | Reference-counted; cancel build when no consumer remains |
| Decoded detail | projection revision + page id | Byte-budget LRU; visible/focused pages pinned only for their frame/job |
| Text layout | string id + font/scale bucket | Bounded LRU; dictionary bytes are not copied |
| Detailed draw data | options + quantized transform + visible page revisions | Small most-recent cache |
| Density/minimap | revision + quantized 2-D viewport + output size + density options | Byte-budget LRU; old image may preview animation |
| Search | revision + regex/options | Current and last-valid result per tile |
| Statistics | revision + range + clock/classifier/policy | Shared bounded result entries |

The UI thread owns tile/runtime maps and reads immutable snapshots. Workers own
builders, decompression, and scratch memory. Lock scope never includes parsing,
decompression, regex evaluation, statistics, or geometry construction. Worker results
carry source generation, model revision, and job revision; stale results are discarded
even if cancellation arrived too late.

### 10.8 Module boundaries

Pipeline code lives under one `libsurfer/src/konata/` family with one-way dependencies:

* `ftr-parser` owns bytes, CBOR, block metadata, typed FTR records, and no UI;
* `TransactionContainer` owns parser-neutral transaction/event query semantics;
* model/index/layout owns normalized pipeline pages and pure algorithms, with no egui
  or `SystemState` dependency;
* rendering depends on model accessors plus theme/egui primitives;
* view/controller alone depends on tiles, `SystemState`, `Message`, tables,
  cursor/markers, commands, and worker lifecycle; and
* `surver` owns remote production while the client owns page transport, both using a
  versioned wire schema independent of Rust struct layout and egui types.

This keeps the model benchmarkable without a GUI, lets tables reuse it without the
tile, and prevents pipeline conventions from leaking into the simulator-agnostic
parser.

---

## 11. Statistics

Deterministic page reductions make the work chunkable and parallel on native,
Web-Worker/cooperatively sliced on wasm. Partial results combine in page order and
cover fetched/committed counts, cycle coverage
(half-open, per UX §11), per-stage-name duration sums/max/counts, per-thread splits,
flush-run detection (consecutive flushed rows terminated by a committed row), and
classifier counters. Classification: explicit instruction-class attribute when
present, else the pluggable label-regex classifier table (generic + x86-gem5
builtins) with per-row provenance recorded. Flush cause attribution follows the UX
rules (explicit relation/attribute first; preceding-committed heuristic only as
`estimated`, separately counted). `flushed == unknown`, missing retirement data, or a
missing clock produces `unknown` for dependent exact metrics rather than a guess.
Results land in a table tile via an analysis-results spec, as signal analysis already
does.

Region statistics use page min-begin/max-end summaries to prune pages, then apply the
UX's exact half-open predicates to candidate rows and clip stage durations to the
selection. This remains correct for rows that begin before the window but retire
inside it and for non-monotonic timestamps. Cache keys include projection revision,
selection, clock mapping, classifier, and estimated-attribution policy.

---

## 12. Data-quality machinery

Quality is computed once at build time, stored as flag bits (§5.1/§5.2) plus a
per-model `QualityCounters { orphans, multi_parent, unnamed, out_of_range,
end_before_start, begin_regressions, unknown_lanes, missing_rid, duplicate_rid }`,
updated per batch during progressive load. One compact diagnostic locator is retained
per distinct issue/category; panning never regenerates warnings. Multiple parents use
the first relation in recorded order while retaining a diagnostic, and malformed
ranges keep their raw endpoints. The tile-header badge renders the counters;
its popover opens pre-filtered raw tables (existing event/transaction table specs).
Every malformed case in UX §13.1 maps to: keep the recorded data, set a flag, count
it — the renderer only ever *adds* warning treatments, never repositions or drops.
Orphan and multi-parent handling reuses the exact semantics `EventIndex` already
implements for the waveform overlay. A fatal parse error mid-load keeps the model at
its last consistent batch and marks the source incomplete with the parser diagnostic.

Two states are distinguished so the user never faces a plausible-looking blank
canvas (UX §3): *while building*, the tile shows progress with rows so far; *after*
a completed build whose stage count is zero, it shows the explicit "No pipeline
stages found" empty state with links to the raw event table and the quality
popover — the counters explain *why* (all orphans, no conforming events, etc.).

---

## 13. Testing strategy

* **Pure-logic unit tests** (the table subsystem precedent of keeping interaction
  logic in pure functions): typed normalization; recorded-order identity with tied or
  backward time; direct/sorted id indexes; viewport math; LOD selection; diagonal
  compensation; rank/select; envelope/RMQ queries; pyramid folding; anchor resolution;
  page-miss behavior; Esc stack; classifier; half-open region clipping; stale-result
  rejection; and bounded-LRU eviction.
* **Property tests** on randomized synthetic traces: IDs stable under
  hide/filter/progressive load; envelope query = brute-force scan; pyramid counts =
  direct counts; CSR round-trips; paged and in-memory adapters publish equivalent
  canonical pages; search results/order = naive scan under arbitrary worker completion.
* **Snapshot tests** through the existing `egui_skia_renderer` harness
  (`libsurfer/src/tests/snapshot.rs`): state-file-driven scenes per LOD regime, color
  scheme, hide-flushed, arrows, overlay compare, quality badges — waiting on
  `konata_caches_ready()` exactly as tests wait on analog/table caches today.
  Determinism holds because rendering is a pure function of (model, tile state,
  theme). Integration scenes cover local, shared-byte, wasm-fallback, and mocked
  capable/incapable Surver sources; focus round trips; reload preservation; and forced
  detail-page eviction.
* **Benchmarks**: extend `ftr-parser/examples/events_bench.rs` (which already
  synthesizes pipeline traces of arbitrary size) to generate the checked-in sample,
  100 k, 1 M, and eviction-forcing multi-M tiers plus adversarial non-monotonic time,
  long stages, dense dependencies, duplicate ids, and all-match search. Record header
  scan, rows-to-first-publication, full build, cold/warm decode, detailed geometry,
  density cold/cached, pan/zoom latency, hit-test, search/cancellation, statistics,
  reload reuse, and peak/resident memory. Numbers accompany the feature per UX §13.3
  and the repository's measured-claims rule.
* **Malformed-input tests**: fixtures for every §12 category, asserting both counters
  and rendering flags.

---

## 14. Phasing

1. **P0 — measurement and convention lock:** freeze the pipeline/clock convention
   version, extend the synthetic generator through million-scale and malformed cases,
   and capture current Surfer/upstream-Konata baselines. *Gate:* reproducible datasets
   and metrics exist before optimization claims.
2. **P1 — shared projection and functional tile:** in-memory query adapter,
   normalizer, immutable row/detail pages, quality flags, tile/state/message plumbing,
   detailed/strip LOD, labels/ruler/navigation/hit testing, focus/cursor/markers,
   themes, and restore. *Gate:* the checked-in sample satisfies core UX and frame work
   is viewport-bounded; no million-scale claim yet.
3. **P2 — scale foundation:** platform-independent `u64` FTR identities, complete
   `BlockMeta`, paged query backend for local/shared bytes, background decode,
   bounded-memory relation joins, decoded-page budgets/prefetch, density/minimap, and
   native/Web-Worker/cooperative executors. *Gate:* 1 M and multi-M traces meet the §1
   memory/interaction budgets while forced eviction is active. This is the first phase
   allowed to claim large-trace support.
4. **P3 — exploration workflows:** arrows/dependency walk/producer chain, cancellable
   regex find and find-to-table, statistics, model-backed lazy tables, commands,
   bookmarks, comparison, sync, and overlay. *Gate:* long work cannot block or
   stale-update the UI; local analysis UX rows are automated.
5. **P4 — remote and production hardening:** versioned Surver transaction pages, WCP,
   append/reload reuse, accessibility, malformed states, cache telemetry, and
   published benchmarks. *Gate:* local, byte-backed, wasm, and capable Surver sources
   have equivalent semantics and the complete UX matrix is automated.

Each phase leaves `main` shippable. P1 intentionally uses the loaded-container adapter;
P2 replaces the scale path before any scale claim is made.

---

## 15. Feasibility ledger

Design elements ↔ verified code anchors (current tree):

| Claim used by this design | Anchor |
|---|---|
| Pane extension point; tile persistence | `SurferPane` enum, `SurferTileTree` — `libsurfer/src/tiles.rs`; `egui_tiles 0.15` serde in workspace `Cargo.toml` |
| Three-way state split + async cache protocol to copy | `TableTileState`/`TableModelSpec` (`table/model.rs`), `TableRuntimeState`/`TableCacheKey`/`TableCacheEntry` (`table/cache/state.rs`), `table_inflight` (`system_state.rs`), `handle_build_table_cache`/`handle_table_cache_built` (`table_controller.rs`) |
| Off-thread build + notify pattern; wasm degradation | `perform_work`, `spawn!` (`async_util.rs`); `OUTSTANDING_TRANSACTIONS` repaint loop (`view.rs`, `channels.rs`) |
| Blocked RMQ precedent (adapted for row-range min/max) | `SignalRMQ`, `AnalogSignalCache` + generation-keyed async build (`analog_signal_cache.rs`, `analog_renderer.rs`, `wave_data.rs`) |
| Two-tier draw caching; pixel-coalescing; truncate-before-layout; per-tx widget anti-pattern | `SystemState.draw_data` (`system_state.rs`), `CachedDrawData` (`lib.rs`); `invalidate_draw_commands`, `generate_transaction_draw_commands_for_source` (binary search by end time, same-pixel skip, event clusters), `draw_region`, per-tx `allocate_rect` (`drawing_canvas.rs`) |
| Parser scalability baseline: u64 times, interning, single-copy indexed relations, lazy stream bodies, eager relation chunks | `ftr-parser/PERFORMANCE.md`; `Transaction`/`Event`/`Attribute`/`TxRelation`, `str_dict`, `tx_block_ids`, `rel_by_source`/`rel_by_sink` declared on `FTR` (`ftr-parser/src/types.rs`; populated in `ftr_parser.rs`) |
| Generator-pair discovery, parent resolution, orphan/multi-parent semantics | `EventIndex` (`libsurfer/src/transaction_events.rs`) |
| FTR load is currently synchronous; bytes path parses eagerly | `load_transactions_from_file`/`_from_bytes` (`wave_source.rs`); `parse_ftr`/`parse_ftr_from_bytes` (`ftr-parser/src/parse.rs`) |
| Shared cursor/markers/focused-transaction cross-view state | `WaveData.cursor/markers/focused_transaction` (`wave_data.rs`); `Message::CursorSet`, `Message::FocusTransactionFromSource` (`message.rs`); `TableAction` → `apply_table_action` (`lib.rs`) |
| Command and WCP registration points | `command_parser.rs::get_parser` + `fzcmd.rs`; `WcpCommand` (`surfer-wcp/src/proto.rs`) + `wcp_handler.rs` (incl. manual greeting list) |
| Viewport easing to reuse | `ViewportStrategy::EaseInOut` (`viewport.rs`) |
| Snapshot harness + wait-for-async-caches idiom | `render_and_compare`, `analog_caches_ready`/`table_caches_ready` wait (`tests/snapshot.rs`) |
| Frame instrumentation with fps reference lines | `benchmark.rs` (`performance_plot` feature) |
| Timescale exponent quirk (4 steps/unit) | `get_timescale` (`ftr-parser/src/types.rs`) |
| Synthetic pipeline-trace generator for benchmarks | `ftr-parser/examples/events_bench.rs` |

New things this design introduces (do not exist today, called out in text):
`TransactionContainer` query facade, complete `BlockMeta`, paged `KonataModel` +
builder and bounded caches (§5–6), parser block visitor and `Arc` body snapshots
(§6.2–6.3), columnar/streamed relations (§6.4), `KonataViewport` (§4.3),
`SurferPane::Konata` + controller + messages (§10), Konata theme keys (§10.6),
Web-Worker execution, progressive FTR loading (§6.3), and the versioned Surver
transaction-page protocol (§10.6).

---

## 16. Risks and open questions

### 16.1 Rejected shortcuts

| Design | Reason rejected |
|---|---|
| Reuse waveform transaction draw commands | Wrong Y projection, time-order assumptions, per-viewport maps, and no deep density hierarchy |
| Keep the generic FTR graph plus another fully resident stage graph | Duplicates the dominant memory; acceptable only for the small-trace adapter |
| Rebuild `EventIndex` after every batch | Repeated O(N + E) work and allocation churn |
| Sort rows by timestamp | Violates recorded-order identity and progressive publication |
| Sample every Nth row at deep zoom | Silently loses short stalls, flush wedges, and other narrow anomalies |
| Synchronously decode on a page miss | Recreates the original Konata hitch; render summaries/placeholders and prefetch |
| Infer a clock from fetch spacing | Bubbles and multi-fetch make it semantically false |
| Treat whole-file raw-URL download as scaled remote support | Retains/transfers the whole trace and has no revisioned page capability |
| Start with a GPU-only renderer | Raises native/wasm/test integration cost before profiles show CPU mesh batching is the bottleneck |

### 16.2 Open risks

* **Relation memory at the multi-million tier** is the largest known parser-side
  scale risk (§6.4). Mitigation is columnar/streamed relations plus permutation-array
  indexes, benchmarkable in isolation via `events_bench`.
* **Clock-mapping discovery** depends on what the convention/`konata2ftr` finally
  emits (§4.2); the model isolates this behind `Option<ClockMapping>` and the UX
  already specifies the manual fallback, so the risk is cosmetic.
* **wasm ceilings**: Web Workers move work off the UI thread but do not remove the
  wasm32 4 GiB address-space ceiling; platforms without workers use time slicing.
  Capacity and latency therefore need their own measured web acceptance tier. The
  architecture keeps every long operation chunked so loss of workers degrades
  throughput, not responsiveness.
* **Remote scale** depends on new Surver capabilities (§10.6), not merely client code.
  Until that protocol exists, whole-file download is the explicit non-scaled fallback.
* **Draw-command regeneration during animation** is per-frame O(screen); if profiling
  ever shows it hot at 4K + strips regime, the mitigation (split the cache into
  viewport-dependent and viewport-independent halves) fits inside §7.1 without
  changing the model.
* **BigInt boundary**: cursor/marker times are `BigInt` globally; the Konata canvas
  clamps to the model's u64 tick domain at the drawing boundary. No precision issue
  inside one trace (writers emit u64), but conversions are centralized in the
  viewport to keep that invariant auditable.

---

## 17. UX requirements trace

| UX area | Architectural coverage |
|---|---|
| §1 goals/principles | Measured invariants/gates (§1), quality preservation (§12), phase gates (§14) |
| §2 data model/clock | Recorded-order identity, typed normalization, explicit clock resolution (§4) |
| §3 opening | Provisional pair discovery, progressive validation, entry points (§6, §10.4) |
| §4–5 anatomy/canvas/LOD | Two-axis viewport, paged columns/CSR, exact aggregates, bounded meshes (§4–§7) |
| §6 navigation/bookmarks | Pointer anchoring, diagonal follow, animation, identity restore (§4.3, §8) |
| §7 inspection/find | Arithmetic hit testing, lazy tooltips, deterministic cancellable search (§7.1, §8–§9) |
| §8 dependencies | Paged dual CSR, endpoint fallbacks, bounded arrows and walks (§5.5, §7.5) |
| §9 colors/hide/compare/minimap | Visibility rank/select, palette compiler, alignment/sync, pyramid minimap (§5.7, §7.7–§7.8, §10.6) |
| §10 Surfer integration | Shared state/focus/time, tables, source tiers, Surver, WCP (§10) |
| §11 statistics | Deterministic typed page reductions and exact half-open regions (§11) |
| §12 options/themes/persistence | Serialized/runtime split, theme revisions, explicit cache keys (§10.2, §10.6–§10.7) |
| §13 robustness/accessibility | Quality diagnostics, visible-only semantic nodes, automated tests (§7.10, §12–§13) |
| §14–15 scenarios/parity | Shared projection workflows and gated delivery/automation (§9–§14) |
