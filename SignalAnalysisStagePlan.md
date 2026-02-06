# Signal Analysis - Staged Implementation Plan

This plan implements the requirements in `SignalAnalysis.md` in small, regression-safe stages.
Rule for all stages: do not start the next stage until the current stage passes the stage gate, status is updated in this document, and changes are committed.

## Current Status Snapshot (2026-02-06)

- Stage 1: Completed (2026-02-06).
- Stage 2: Completed (2026-02-06).
- Stage 3: Completed (2026-02-06).
- Stage 4: Completed (2026-02-06).
- Stage 5: Completed (2026-02-06).
- Stage 6: Completed (2026-02-06).
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

Status: Completed (2026-02-06).

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

Validation status (2026-02-06):
- `ulimit -n 10240`: applied before each gate command.
- `cargo fmt`: pass.
- `cargo clippy --no-deps`: pass (existing warning in `libsurfer/src/table/cache.rs` about `type_complexity`).
- `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.
- `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.

Implemented:
- Added `SignalAnalysisResultsModel` in `libsurfer/src/table/sources/signal_analysis.rs`, reusing Stage 2 kernel helpers for trigger extraction, marker intervals, and metric accumulation.
- Implemented full `TableModel` behavior for analysis results: schema, rows/cells, sort keys, search text, and row activation (`CursorSet` to interval end timestamp).
- Added required-signal availability checks so model creation returns `TableCacheError::DataUnavailable` when sampling/analyzed signals are not loaded.
- Wired `TableModelSpec::create_model` for `AnalysisResults { SignalAnalysisV1 }` and added analysis-specific default view config (title, sort, single-select with activate-on-select).
- Added Stage 3 table tests covering schema/row/cell/sort/search/activation, interval-end activation semantics, non-empty field handling, and default view config behavior.

## Stage 4 - Run Path and Async Cache Integration

Goal: Add message and runtime flow to create analysis tile and build cache asynchronously.

Status: Completed (2026-02-06).

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

Validation status (2026-02-06):
- `ulimit -n 10240`: applied before each gate command.
- `cargo fmt`: pass.
- `cargo clippy --no-deps`: pass (existing warning in `libsurfer/src/table/cache.rs` about `type_complexity`).
- `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.
- `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.

Implemented:
- Added `Message::RunSignalAnalysis { config }` and runtime handling in `SystemState::update`.
- Added signal-analysis preflight loading for sampling/analyzed signals before model creation, using the existing waveform signal-loading path.
- Added run-path table-tile creation for `TableModelSpec::AnalysisResults { SignalAnalysisV1 }` and routed cache build through existing `BuildTableCache -> TableCacheBuilt` flow.
- Added Stage 4 integration tests for run flow and async cache build flow:
  - `run_signal_analysis_creates_analysis_tile_and_preloads_signals`
  - `signal_analysis_build_table_cache_flow_completes_after_run`

## Stage 5 - Wizard UI and Context Menu Entry

Goal: Add user-facing analysis configuration workflow.

Status: Completed (2026-02-06).

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

Validation status (2026-02-06):
- `ulimit -n 10240`: applied before each gate command.
- `cargo fmt`: pass.
- `cargo clippy --no-deps`: pass (existing warning in `libsurfer/src/table/cache.rs` about `type_complexity`).
- `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.
- `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.

Implemented:
- Added context-menu entry `Analyze selected signals...` (visible only when at least one selected waveform item is a variable) and wired it to `Message::OpenSignalAnalysisWizard`.
- Added signal-analysis wizard UI state (`show_signal_analysis_wizard`) and a single-page dialog with sampling signal selection, resolved sampling-mode display, selectable signal list with per-signal translator dropdowns, marker interval info, and Run/Cancel actions.
- Added keyboard support in the wizard (`Enter` to run when valid, `Escape` to cancel) and disabled global key handling while the wizard is open.
- Added runtime helpers to derive selected analysis signals (filtering out non-variable selections), sampling options from displayed variables, one-bit default sampling selection, and inferred sampling mode display.
- Added Stage 5 tests for menu-visibility/selection filtering behavior and wizard-open defaults:
  - `signal_analysis_menu_visibility_tracks_variable_selection`
  - `open_signal_analysis_wizard_filters_non_variable_selection`
  - `open_signal_analysis_wizard_defaults_sampling_to_first_one_bit_signal`
  - `open_signal_analysis_wizard_requires_selected_variables`
- Added wizard snapshot coverage:
  - `tests::snapshot::signal_analysis_wizard_dialog`
  - baseline image `snapshots/signal_analysis_wizard_dialog.png`.

## Stage 6 - Refresh/Edit and Revisioned Rebuild Semantics

Goal: Support snapshot re-analysis workflow with deterministic rebuilds.

Status: Completed (2026-02-06).

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

Validation status (2026-02-06):
- `ulimit -n 10240`: applied before each gate command.
- `cargo fmt`: pass.
- `cargo clippy --no-deps`: pass (existing warning in `libsurfer/src/table/cache.rs` about `type_complexity`).
- `cargo test -- --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.
- `cargo test -- --include-ignored --skip tests::wcp_tcp:: --skip file_watcher::tests::notifies_on_change --skip file_watcher::tests::resolves_files_that_are_named_differently`: pass.

Implemented:
- Added `Message::RefreshSignalAnalysis { tile_id }` and `Message::EditSignalAnalysis { tile_id }`.
- Added signal-analysis inline table actions (`Refresh`, `Edit`) in `libsurfer/src/table/view.rs`.
- Added edit workflow state to `UserState` and wired edit-run behavior to update the existing analysis tile (instead of creating a new tile), bumping `run_revision` on each edit-run.
- Added `TableModelSpec::model_key_for_tile` and used it for table cache keys so `SignalAnalysisConfig.run_revision` participates in cache/model identity, forcing deterministic refresh rebuilds.
- Hardened stale async handling in `Message::TableCacheBuilt` so stale revisions are ignored without evicting the current in-flight cache entry.
- Added Stage 6 coverage in `libsurfer/src/table/tests.rs`:
  - `signal_analysis_model_key_changes_on_refresh_run_revision`
  - `edit_signal_analysis_run_updates_existing_tile_and_bumps_revision`
  - `stale_signal_analysis_result_does_not_evict_current_inflight_entry`

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
