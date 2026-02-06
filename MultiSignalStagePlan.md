# Multi-Signal Change List - Staged Implementation Plan

This plan implements the requirements in `MultiSignalChangeList.md` in small, regression-safe stages.
Rule for all stages: do not start the next stage until the current stage passes the full test gate.

## Global Stage Gate (run after every stage)

1. `cargo fmt`
2. `cargo clippy --no-deps`
3. `cargo test`
4. `cargo test -- --include-ignored`

If snapshot output changes are expected in a stage, review the diffs, run `./accept_snapshots.bash`, and rerun `cargo test -- --include-ignored` before continuing.

## Stage 1 - Spec and Serialization Skeleton

Goal: introduce the new table spec and config shape without changing runtime behavior.

Status: Completed (2026-02-06)

Implemented:
- Added `TableModelSpec::MultiSignalChangeList { variables: Vec<MultiSignalEntry> }`.
- Added `MultiSignalEntry { variable: VariableRef, field: Vec<String> }`.
- Added default view config for this spec with deterministic title, `time` ascending sort, and single-row activation behavior.
- Kept model creation behind a placeholder `ModelNotFound` path to avoid leaking partial runtime logic.

Scope:
- Add `TableModelSpec::MultiSignalChangeList { variables: Vec<MultiSignalEntry> }`.
- Add `MultiSignalEntry { variable: VariableRef, field: Vec<String> }`.
- Add default view config for this spec with time ascending sort and single-row activation behavior.
- Keep model creation behind a placeholder error path for now so no partial runtime logic leaks in.

Expected files:
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`
- `libsurfer/src/table/mod.rs` (type re-export adjustment)

Tests to add:
- RON round-trip for `TableModelSpec::MultiSignalChangeList`.
- Deterministic default title and default sort behavior for the new spec.
- Error behavior for model creation before implementation exists.

Implemented tests:
- `table_model_spec_multi_signal_change_list_ron_round_trip`
- `multi_signal_change_list_default_view_config_deterministic`
- `multi_signal_change_list_model_creation_is_placeholder_error`

## Stage 2 - TableModel Lazy/Batch API (Backward Compatible)

Goal: add the required materialization contract without breaking existing models.

Scope:
- Extend `TableModel` with batch-oriented APIs:
- `materialize_window(row_ids, visible_cols, purpose)`.
- `MaterializePurpose` enum (`Render`, `SortProbe`, `SearchProbe`, `Clipboard`).
- Keep `cell`, `sort_key`, and `search_text` as compatibility methods with default adapter logic.
- Ensure existing models (`Virtual`, `SignalChangeList`, `TransactionTrace`) compile and behave unchanged.

Expected files:
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/sources/virtual_model.rs`
- `libsurfer/src/table/sources/signal_change_list.rs`
- `libsurfer/src/table/sources/transaction_trace.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- Default adapter path returns identical values to legacy `cell/sort_key/search_text`.
- Existing model tests remain unchanged and pass.

## Stage 3 - Cache Pipeline Refactor for Lazy Search/Sort Probes

Goal: make table cache capable of index-only row storage for models that opt into lazy probing.

Scope:
- Refactor `TableCache` to store row ordering/index as the durable cache core.
- Add lazy search probe path used by filtering and type-to-search when model does not provide eager `search_texts`.
- Keep eager behavior for existing models to avoid regressions.
- Update table view/type-search logic to consume lazy probe path when present.

Expected files:
- `libsurfer/src/table/cache.rs`
- `libsurfer/src/table/view.rs`
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- Filtering correctness for eager models (no behavior change).
- Type-to-search correctness with lazy probe provider.
- Cache shape regression tests verifying row IDs and row index stability.

## Stage 4 - Pure Merged Index Builder (No UI Wiring Yet)

Goal: implement and validate the sparse merged timeline index as a standalone component.

Scope:
- Add `MergedIndex`, `SignalRuns`, and `TransitionAtTime` builder from transition iterators.
- Enforce row identity as `TableRowId(time_u64)` with deduplicated merged timeline.
- Provide `O(log R)` helpers for exact-hit run lookup and previous-run lookup.
- Keep this module independent from UI and translation concerns.

Expected files:
- `libsurfer/src/table/sources/multi_signal_index.rs` (new)
- `libsurfer/src/table/sources/mod.rs`
- `libsurfer/src/table/tests.rs` or dedicated unit test module

Tests to add:
- Global merged timeline dedup/sort correctness.
- Per-signal same-timestamp run grouping correctness.
- Exact run and previous run lookup behavior.
- Duplicate selected signal dedup logic by `(VariableRef, field)`.

## Stage 5 - MultiSignal Model Skeleton with Index-Only Rows

Goal: wire a new model that uses the merged index but still keeps per-cell rendering minimal.

Scope:
- Add `MultiSignalChangeListModel` with:
- signal entry resolution (`VariableRef`, field, translator/meta/accessor).
- lazy `OnceLock<MergedIndex>`.
- schema generation with stable column keys:
- `time`
- `sig:v1:<escaped-full-variable-path>#<escaped-field-path>`
- Implement row count, row id lookup, time column rendering, and row activation cursor set.
- Integrate `TableModelSpec::create_model` with this new model.

Expected files:
- `libsurfer/src/table/sources/multi_signal_change_list.rs` (new)
- `libsurfer/src/table/sources/mod.rs`
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- Model creation with valid signals.
- Missing/unloaded signals skipped with warning, fail if no valid signals remain.
- Stable and reversible column key generation.
- `on_activate` sets cursor to row timestamp.

## Stage 6 - On-Demand Cell Materialization Semantics

Goal: implement `Transition/Held/NoData` behavior using index + `query_variable`, with no full cell matrix.

Scope:
- Implement per-cell materialization from:
- exactness and run length from index.
- value lookup from `query_variable(variable, T)`.
- Render semantics:
- transition text normal.
- held text dimmed.
- no-data as dimmed em dash text.
- collapsed same-time run marker as `(+N)` where `N = run_len - 1`.
- Ensure `cell`, `sort_key`, and `search_text` delegate to on-demand probes.

Expected files:
- `libsurfer/src/table/sources/multi_signal_change_list.rs`
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- `Transition/Held/NoData` classification.
- `query_variable` authoritative value behavior at exact and held timestamps.
- Collapsed count correctness for same-timestamp runs.
- Numeric vs text sort key probing behavior.

## Stage 7 - Window Materialization Cache and Renderer Integration

Goal: optimize viewport rendering to materialize only visible windows and reuse short-lived cached windows.

Scope:
- Implement model-local `WindowCellCache` keyed by:
- row-range bucket.
- visible columns.
- table revision.
- time-format revision.
- translator/format revision.
- waveform generation.
- Update renderer and clipboard paths to call `materialize_window(...)`.
- Keep no-global-cell-table invariant (`Vec<MergedRow { cells: ... }>` must not exist).

Expected files:
- `libsurfer/src/table/sources/multi_signal_change_list.rs`
- `libsurfer/src/table/view.rs`
- `libsurfer/src/table/cache.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- Materialization limited to requested window.
- Cache reuse on repeated viewport requests.
- Cache invalidation on revision/generation changes.
- Clipboard export uses window materialization and preserves visible-column order.

## Stage 8 - Async Revision Gating and Cancellation Safety

Goal: ensure stale async work never commits and new requests supersede old work safely.

Scope:
- Add monotonic `table_revision` per tile runtime.
- Capture revision + task kind in async cache/filter/sort/search work.
- Apply results only if revision still matches active runtime state.
- Add cooperative cancellation checks for long chunked operations.

Expected files:
- `libsurfer/src/table/cache.rs`
- `libsurfer/src/lib.rs`
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- Stale async completion ignored after sort/filter change.
- Revision increment behavior on each superseding request.
- Selection/scroll behavior remains correct across canceled and superseded tasks.

## Stage 9 - UX Entry Point and Drill-Down Behavior

Goal: complete user-facing access path while preserving existing single-signal UX.

Scope:
- Context menu behavior:
- exactly one variable selected -> `Signal change list`.
- two or more variables selected -> `Multi-signal change list`.
- non-variable selected items ignored.
- Add message/spec creation path for multi-signal list tile.
- Preserve drill-down from a signal column to single-signal change list.

Expected files:
- `libsurfer/src/menus.rs`
- `libsurfer/src/message.rs`
- `libsurfer/src/lib.rs`
- `libsurfer/src/table/tests.rs`

Tests to add:
- Menu action selection for one vs many variable selections.
- Non-variable selections do not create invalid multi-signal entries.
- Drill-down action opens correct single-signal table spec.

## Stage 10 - Snapshot, Performance, and Hardening Pass

Goal: validate final UX and memory/performance constraints from the spec.

Scope:
- Add snapshot coverage for:
- held-value dimming.
- collapsed `(+N)` marker.
- large-table scrolling visuals.
- Add integration tests for end-to-end flow:
- `AddTableTile` -> async build -> render.
- async sort/filter/search behavior with lazy probes.
- cursor activation behavior.
- Add a synthetic stress test fixture for 10 signals x 100K transitions and assert no full cell matrix allocation patterns.
- Final docs/comments cleanup.

Expected files:
- `libsurfer/src/tests/snapshot.rs`
- `libsurfer/src/table/tests.rs`
- `libsurfer/src/table/sources/multi_signal_change_list.rs`
- `MultiSignalChangeList.md` (only if clarifications are needed)

Tests to add:
- Integration and snapshot tests listed in section 14 of `MultiSignalChangeList.md`.
- Regression tests for memory-safe sparse model behavior.

## Implementation Discipline Rules

1. Keep each stage small enough for one focused PR/commit set.
2. Do not mix behavior changes from future stages into earlier stages.
3. If a stage reveals an architectural blocker, stop and record a redesign note before coding around it.
4. Preserve existing table behavior for `SignalChangeList`, `TransactionTrace`, and `Virtual` at every stage.
