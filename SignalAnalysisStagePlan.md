# Signal Analysis - Staged Implementation Plan

This plan implements the requirements in `SignalAnalysis.md` in small, regression-safe stages.
Rule for all stages: do not start the next stage until the current stage passes the stage gate, status is updated in this document, and changes are committed.

## Current Status Snapshot (2026-02-06)

- Stage 1: Completed (2026-02-06).
- Stage 2: Completed (2026-02-06).
- Stage 3: Not started.
- Stage 4: Not started.
- Stage 5: Not started.
- Stage 6: Not started.
- Stage 7: Not started.

## Global Stage Gate (run after every stage)

1. `ulimit -n 10240`
2. `cargo fmt`
3. `cargo clippy --no-deps`
4. `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`
5. `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`

Authoritative (unrestricted) validation:
- `cargo test`
- `cargo test -- --include-ignored`

## Mandatory Per-Stage Completion Checklist

1. Finish the stage scope only.
2. Run the full Global Stage Gate.
3. Update this file:
   - Mark the stage as `Completed (YYYY-MM-DD)`.
   - Add a short validation status block with gate results.
   - Add a brief "Implemented" summary.
4. Commit changes for that stage before starting next stage.
   - Recommended commit subject format: `SignalAnalysis Stage <N>: <short summary>`
5. Move to the next stage only after commit is created.

## Stage 1 - Analysis Spec and Serialization

Goal: Replace placeholder analysis spec with typed Signal Analysis config that round-trips in RON state.

Status: Completed (2026-02-06).

Scope:
- Add typed `SignalAnalysisConfig` and related structs/enums in table model types.
- Extend `AnalysisKind` and `AnalysisParams` for `SignalAnalysisV1`.
- Keep runtime behavior unchanged (model may still be unimplemented at this stage).

Expected files:
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/mod.rs`
- `libsurfer/src/table/tests.rs`

Exit criteria:
- RON round-trip tests for analysis spec/config pass.
- No behavior regressions in existing table specs.

Validation status (2026-02-06):
- `ulimit -n 10240`: applied before each gate command.
- `cargo fmt`: pass.
- `cargo clippy --no-deps`: pass (existing warning in `libsurfer/src/table/cache.rs` about `type_complexity`).
- `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.
- `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.

Implemented:
- Added typed signal analysis table spec types: `SignalAnalysisConfig`, `SignalAnalysisSamplingConfig`, `SignalAnalysisSignal`, and `SignalAnalysisSamplingMode`.
- Extended analysis selectors with `AnalysisKind::SignalAnalysisV1` and `AnalysisParams::SignalAnalysisV1 { config }` while preserving placeholder support.
- Added RON round-trip tests for typed signal analysis config and analysis table spec variants.

## Stage 2 - Pure Computation Kernel

Goal: Implement and test pure logic for mode inference, trigger extraction, intervals, and metric accumulation.

Status: Completed (2026-02-06).

Scope:
- Add new source module for signal analysis computation helpers.
- Implement:
  - Sampling mode inference (`Event`, `PosEdge`, `AnyChange`)
  - Trigger collection
  - Marker normalization and interval construction
  - Per-interval/global accumulator math
  - Non-numeric and empty-interval handling

Expected files:
- `libsurfer/src/table/sources/signal_analysis.rs` (new)
- `libsurfer/src/table/sources/mod.rs`

Exit criteria:
- Unit tests cover all algorithmic edge cases from `SignalAnalysis.md`.

Validation status (2026-02-06):
- `ulimit -n 10240`: applied before each gate command.
- `cargo fmt`: pass.
- `cargo clippy --no-deps`: pass (existing warning in `libsurfer/src/table/cache.rs` about `type_complexity`).
- `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.
- `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.

Implemented:
- Added pure signal-analysis kernel module `libsurfer/src/table/sources/signal_analysis.rs`.
- Implemented mode inference, trigger extraction, BigInt->u64 run-range normalization, marker normalization, interval construction, and per-interval/global metric accumulation.
- Added unit coverage for all stage-2 algorithm edges: mode precedence, posedge detection, marker sort/dedup/clipping, interval boundary semantics, NaN/non-numeric handling, empty intervals, and out-of-range triggers.
- Re-exported stage-2 kernel APIs from `libsurfer/src/table/sources/mod.rs` for Stage 3 integration.

## Stage 3 - SignalAnalysis TableModel

Goal: Build `TableModel` implementation for analysis results using Stage 2 kernel.

Status: Not started.

Scope:
- Implement schema, rows, cells, sort keys, search text, and `on_activate`.
- Implement row activation to set cursor to interval end timestamp.
- Return `DataUnavailable` when required signals are not loaded.
- Wire `TableModelSpec::create_model` to instantiate the analysis model.

Expected files:
- `libsurfer/src/table/sources/signal_analysis.rs`
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`

Exit criteria:
- Table model compliance tests pass for schema/row/cell/sort/search/activation.

## Stage 4 - Run Path and Async Cache Integration

Goal: Add message and runtime flow to create analysis tile and build cache asynchronously.

Status: Not started.

Scope:
- Add `RunSignalAnalysis` message path.
- Preflight requested signals loading before model creation.
- Create table tile with `TableModelSpec::AnalysisResults`.
- Ensure async build goes through existing `BuildTableCache -> TableCacheBuilt` flow.

Expected files:
- `libsurfer/src/message.rs`
- `libsurfer/src/lib.rs`
- `libsurfer/src/table/tests.rs`

Exit criteria:
- Integration tests pass for run flow and cache-build flow.

## Stage 5 - Wizard UI and Context Menu Entry

Goal: Add user-facing analysis configuration workflow.

Status: Not started.

Scope:
- Add waveform context-menu action: `Analyze selected signals...`.
- Add `OpenSignalAnalysisWizard` message and UI state.
- Implement single-page wizard dialog with:
  - Sampling signal selection
  - Read-only resolved mode display
  - Signal list with checkbox + translator
  - Marker info line
  - Run/Cancel actions and validation

Expected files:
- `libsurfer/src/menus.rs`
- `libsurfer/src/message.rs`
- `libsurfer/src/state.rs`
- `libsurfer/src/view.rs`
- `libsurfer/src/dialog.rs` (or dedicated dialog module)
- `libsurfer/src/tests/snapshot.rs`

Exit criteria:
- Snapshot tests for wizard rendering pass.
- Menu visibility and selection filtering behavior tested.

## Stage 6 - Refresh/Edit and Revisioned Rebuild Semantics

Goal: Support snapshot re-analysis workflow with deterministic rebuilds.

Status: Not started.

Scope:
- Add `RefreshSignalAnalysis` and `EditSignalAnalysis` messages.
- Add table-tile inline actions for refresh and edit.
- Use `run_revision` in cache/model identity so refresh always rebuilds.
- Guard against stale async results from older revisions.

Expected files:
- `libsurfer/src/message.rs`
- `libsurfer/src/lib.rs`
- `libsurfer/src/table/view.rs`
- `libsurfer/src/table/model.rs`
- `libsurfer/src/table/tests.rs`

Exit criteria:
- Tests verify refresh forces rebuild and stale results are ignored.

## Stage 7 - Final Coverage, Snapshots, and Docs

Goal: Finalize tests, snapshots, and documentation for stable rollout.

Status: Not started.

Scope:
- Add/complete integration tests for serialization, activation, and refresh semantics.
- Add/refresh result-table snapshots.
- Update relevant docs/comments and ensure naming/titles match UX spec.

Expected files:
- `libsurfer/src/table/tests.rs`
- `libsurfer/src/tests/snapshot.rs`
- `SignalAnalysis.md` (only if spec clarifications are required)

Exit criteria:
- Stage gate green.
- Unrestricted authoritative test runs green.
- Plan status updated to reflect completion.
