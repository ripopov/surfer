# Table Widget Performance Plan (Revised)

This document updates the previous plan using the current codebase state in `libsurfer/src/table/*` and `libsurfer/src/lib.rs`.

Primary goal: keep table interaction responsive for very large datasets (up to millions of rows) by removing steady-state O(n) UI-thread work.

## Success criteria

1. No per-frame O(n) work on the UI thread in steady state.
2. Table model is not rebuilt per frame or per click.
3. Cache rebuilds are async and stale results are safely discarded.
4. Architecture remains testable, maintainable, and explicit about invalidation rules.

## Verified current state (code snapshot)

The following issues are present now:

1. Model creation on every frame in `draw_table_tile()` (`libsurfer/src/table/view.rs:57`).
2. Model creation on selection/copy/activation paths (`libsurfer/src/lib.rs:2473`, `libsurfer/src/lib.rs:2510`, `libsurfer/src/lib.rs:2546`).
3. `BuildTableCache` still creates the model on the UI thread before spawning worker (`libsurfer/src/lib.rs:2261`).
4. Per-frame clone of all row IDs in rendering (`libsurfer/src/table/view.rs:502`).
5. Per-frame clone of all row IDs in filter bar (`libsurfer/src/table/view.rs:821`).
6. Keyboard handling clones `row_ids` and `search_texts` (`libsurfer/src/table/view.rs:260-261`).
7. `count_hidden()` builds a `BTreeSet` every time (`libsurfer/src/table/model.rs:303`), called from filter bar (`libsurfer/src/table/view.rs:827`).
8. Scroll and selection helpers still do linear scans on `row_ids` (`libsurfer/src/table/view.rs:484`, `libsurfer/src/table/model.rs:384`, `libsurfer/src/table/model.rs:604`, `libsurfer/src/table/model.rs:844`, `libsurfer/src/table/model.rs:904`).
9. `TableCache` stores `sort_keys` permanently even though they are only needed during sort (`libsurfer/src/table/cache.rs:127`, `libsurfer/src/table/cache.rs:434`).
10. `TableCacheKey` does not encode all model-context dependencies (time format/unit and translator/config driven model output can drift without explicit model invalidation).

## Architecture decisions

### 1. Separate model lifecycle from view-cache lifecycle

Treat model and cache as two layers:

1. **Model layer**: expensive source-dependent object (`Arc<dyn TableModel>`), keyed by `ModelBuildKey`.
2. **View cache layer**: filtered/sorted visible row order and lookup indexes, keyed by `TableCacheKey`.

`TableRuntimeState` should own both and avoid reconstructing model on read paths.

### 2. Add explicit revision keys for correctness

Introduce model invalidation key that is independent from filter/sort:

```rust
pub struct ModelBuildKey {
    pub tile_id: TableTileId,
    pub generation: u64,            // waveform generation
    pub context_revision: u64,      // time format/unit, translators, format-related deps
}
```

`TableCacheKey` remains view-specific (filter/sort + generation), but cache results must be tied to the model key that built them.

### 3. Keep async pipeline stale-safe

Worker results must include both keys and be applied only if keys still match runtime state.

This prevents race bugs when user changes filter/sort/spec while work is in flight.

### 4. Eliminate steady-state cloning

UI render and keyboard paths borrow slices and shared references. No `clone()` of million-entry vectors in hot paths.

### 5. Prefer low-risk wins before deep refactors

Do no-regret fixes first (clone removal, indexes, handler reuse), then move model construction async where it has the highest impact.

## Target data structures

### `TableRuntimeState`

Add model and lightweight derived stats:

```rust
pub struct TableRuntimeState {
    pub cache_key: Option<TableCacheKey>,
    pub cache: Option<Arc<TableCacheEntry>>,
    pub model: Option<Arc<dyn TableModel>>,          // NEW
    pub model_key: Option<ModelBuildKey>,            // NEW
    pub hidden_selection_count: usize,               // NEW (derived)
    pub last_error: Option<TableCacheError>,
    pub selection: TableSelection,
    pub scroll_offset: f32,
    pub type_search: TypeSearchState,
    pub scroll_state: TableScrollState,
    pub filter_draft: Option<FilterDraft>,
}
```

### `TableCache`

```rust
pub struct TableCache {
    pub row_ids: Vec<TableRowId>,
    pub row_index: std::collections::HashMap<TableRowId, usize>, // NEW
    pub search_texts: std::sync::Arc<[String]>,                   // keep for now, avoid clones
    // sort_keys removed from final cache
}
```

Notes:

1. `sort_keys` should remain temporary inside `build_table_cache()` only.
2. `search_texts` is kept initially for behavior parity and simpler migration; memory reduction can be stage 4.

## Build pipeline (target)

### Request path

1. UI computes `model_key` and `cache_key`.
2. If model missing/stale, queue model+cache build.
3. If model valid but cache stale, queue cache-only build.

### Worker path

1. Build (or reuse provided) model.
2. Build filtered/sorted cache from model.
3. Return `TableCacheBuilt` with `entry`, `model`, `model_key`, `cache_key`, `result`.

### Apply path

1. Ignore result if runtime keys changed.
2. Store model/cache on match.
3. Recompute `hidden_selection_count` once.

## Implementation plan

### Phase 0: Safety rails and baseline

1. Add targeted tests for table runtime transitions (stale result discard, key matching, model/cache invalidation).
2. Add debug timing logs around model build and cache build to validate improvements.

### Phase 1: No-regret hot-path fixes (do first)

1. Remove row ID clones in `render_table()` and filter bar.
2. Refactor keyboard handling into read/apply phases using borrowed slices.
3. Add `row_index` to cache and use it for:
   - `scroll_to_row`
   - selection range helpers where possible
   - hidden/visible count fast paths
4. Remove `sort_keys` from `TableCache` output.
5. Replace per-frame `count_hidden()` work with cached `hidden_selection_count` updated on selection/cache changes.

Expected result: immediate frame-time and allocation reduction without changing model lifecycle yet.

### Phase 2: Model caching and handler cleanup

1. Add `runtime.model` and `runtime.model_key`.
2. Stop calling `create_model()` in:
   - `draw_table_tile()`
   - `SetTableSelection` activation path
   - `TableActivateSelection`
   - `TableCopySelection`
3. Use cached model for activation/copy behavior.
4. Add `context_revision` source in `SystemState` and include it in `ModelBuildKey`.

Expected result: no model rebuild during steady-state interactions.

### Phase 3: Async model construction (heavy models first)

1. Introduce owned build input for worker-side model construction:

```rust
pub enum TableModelBuildInput {
    Virtual { rows: usize, columns: usize, seed: u64 },
    SignalChangeList { /* owned snapshot fields */ },
    TransactionTrace { /* owned snapshot fields */ },
}
```

2. Build input creation happens on UI thread and must be cheap.
3. Heavy model materialization moves to worker.
4. `BuildTableCache` no longer calls `create_model()` directly.

Design note: if a model cannot yet produce a cheap owned build input, keep a temporary fallback (model built on UI thread once per `model_key`) and migrate model-by-model.

### Phase 4: Search memory/perf tuning

1. Keep API stable, then replace `search_texts` backing storage if needed:
   - `Arc<[Box<str>]>`, or
   - prefix index + fallback full search.
2. Make type-to-search allocation-free on per-row path.

This phase is optional until memory/profile data shows it is needed.

### Phase 5: Optional selection semantics improvements

1. Revisit `Ctrl+A` complexity only if still problematic.
2. Consider virtual select-all representation only with clear semantics/tests (clipboard/export behavior, hidden count correctness, filter interactions).

Do not introduce `select_all` flag before phases 1-3 are complete and benchmarked.

## Invalidation rules (must be explicit)

1. **Rebuild model + cache** when `ModelBuildKey` changes.
2. **Rebuild cache only** when filter/sort changes and model key is unchanged.
3. **Selection-only recompute** for hidden count when selection changes.
4. **Drop stale worker results** when keys mismatch.

Events that should bump `context_revision` include at least:

1. Time unit/time formatting changes.
2. Translator set changes.
3. Display-format changes that affect table model output (for signal-change-list backed tables).

## Testing strategy (automated only)

1. Unit tests for key logic:
   - `ModelBuildKey` / `TableCacheKey` matching behavior
   - hidden count updates
   - row index lookup paths
2. Message-flow tests:
   - stale async result ignored
   - latest result applied
   - activation/copy works without `create_model()` in handlers
3. Regression tests for keyboard and filter behavior with large virtual tables.
4. Keep existing snapshot tests unchanged for visual parity.
5. Add ignored performance regression tests for 1M-row virtual tables focusing on:
   - no per-frame allocation spikes
   - no repeated model construction during interaction

## Risks and mitigations

1. **Stale async apply**: solved by strict key matching on apply.
2. **Over-invalidation**: start conservative (correctness first), then narrow triggers.
3. **Model-specific migration complexity**: migrate per model type with fallback path.
4. **Memory cost of `row_index`**: accepted tradeoff for interaction latency; profile after phase 1.

## Definition of done

1. Rendering path does not clone full row vectors or rebuild model.
2. Activation/copy/selection handlers do not call `create_model()`.
3. `BuildTableCache` path supports worker-side model build for heavy sources (or temporary one-time model fallback with no per-frame rebuild).
4. All table tests pass, and new key/race tests are added.
5. 1M-row virtual-table interaction remains responsive under filter/sort/navigation operations.
