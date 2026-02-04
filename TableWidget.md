# Table Widget (Table Tile) Design Document

This document proposes the Table widget (Table tile) for Surfer. It focuses on a scrollable, sortable, searchable, selectable, responsive, and accessible table UI, with a clean model/view separation and RON serialization of layout/configuration only.

## Usage scenarios

- Signal change list: show <time, value> transitions for a single signal, with row selection jumping the cursor to the chosen time.
- FTR transaction trace: one transaction per row with columns for type, start/end, duration, generator, attributes; selection updates focused transaction.
- Signal search results: one row per occurrence of a pattern/value, with click-to-jump to the time and optional highlight of the signal.
- Signal analysis results: derived metrics (top 10 spikes, longest idle windows, glitch detection) presented in table form.
- Virtual data model: synthetic rows/columns for UI testing, benchmarks, and demoing table features.
- Future sources: any derived view over waveform/transaction data (statistics, exports, rule matches, protocol decodes).

## Design overview

The Table widget is implemented as a new tile type in the existing egui_tiles layout. Each table tile consists of:

- A serializable configuration (model spec + view config) stored in UserState, allowing layout persistence in .ron state files.
- A runtime model and cache built on demand from the currently loaded waveform/transaction data. This cache is not serialized and is rebuilt after state load or data reload.
- A view layer that renders the table and handles user interaction (scroll, sort, search, selection), producing Messages to mutate state.

Key capabilities:

- Scrollable (vertical and horizontal) with virtualized rows.
- Sortable by column with ascending/descending and stable ordering.
- Searchable with filter modes and live feedback.
- Selectable with mouse and keyboard navigation.
- Responsive layout (resizable columns, narrow-width behavior).
- Accessible via egui widgets and accesskit when enabled.

This closely mirrors the analog signal rendering pipeline:

- Analog settings are serialized; caches are not.
- A cache entry is built asynchronously and shared via Arc.
- A generation counter invalidates caches after waveform reload.
- The view shows a loading state while cache builds.

## Design decisions

- Model/view separation: data sources implement a TableModel trait; the table UI is a TableView that depends only on the trait and a cached index.
- Serializable config only: table tiles store a TableModelSpec and TableViewConfig in .ron files. No table row data is serialized. This matches analog_signal_cache and avoids bloating state files.
- Async caching with invalidation: expensive operations (sorting, search, row index construction) run in a worker thread. Cache entries are keyed by (model identity + display_filter + sort + generation) and invalidated when wave data reloads.
- In-flight dedupe: at most one cache build per (tile_id, cache_key) runs at a time.
- Cache adoption guard: TableCacheBuilt is only applied if (tile_id exists, cache_key matches current view state, and generation matches). Stale results are dropped to avoid UI races after sort/filter changes or tile close.
- Stable row identity: selection uses a stable TableRowId (not an index) so selections survive sorting/filtering and incremental updates. Selection persists for filtered-out rows.
- Selection invalidation on reload: when cache_generation changes, selection is cleared (or remapped if a model provides a remap hook in a future version).
- Single source of truth: table data is derived from WaveData/TransactionContainer. Tables do not own or mutate waveform data.
- Threading contract: TableModel implementations must be Send + Sync and backed by immutable data (Arc snapshots or read-only handles) so cache builds can run off-thread without borrowing UI thread state.
- Column identity: TableSchema defines stable column keys; TableColumnConfig and TableSortSpec reference keys (not indices). Unknown keys are ignored; missing columns use schema defaults.
- Consistent formatting: time and value columns reuse existing formatting utilities (TimeStringFormatting, TimeUnit, translators) to match the waveform view.
- Accessibility: table rows are focusable and keyboard navigable. The design uses egui widgets with accesskit support when the feature is enabled.
- Performance first: rendering is virtualized (only visible rows are drawn), and searching/sorting is cached and incremental to avoid per-frame O(n) work.
- Undo/redo safety: runtime caches are excluded from Clone/serde, similar to AnalogVarState. Table config changes can be undoable, but caches must be rebuilt.
- Tile lifecycle: table tiles are closable; closing a tile removes its config and runtime cache and prunes the tile tree.
- Column formatting (v1): no per-column alignment/format configuration beyond schema defaults; TableColumnConfig only covers layout behavior (width, visibility, resizing).
- Selection state is runtime-only (not serialized); TableViewConfig only stores the selection_mode. Scroll position is also runtime-only.

## API

### New types

```rust
/// Unique identifier for a table tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableTileId(pub u64);

/// Stable row identity for selection and caching.
///
/// Row identity contract:
/// - SignalChangeList: timestamp of the transition
/// - TransactionTrace: transaction ID from FTR
/// - SearchResults: underlying model's row ID
/// - Virtual: row index (stable within session)
///
/// Models MUST provide stable IDs within a generation; IDs may change across reloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableRowId(pub u64);

/// Serializable model selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TableModelSpec {
    SignalChangeList { variable: VariableRef, field: Vec<String> },
    TransactionTrace { stream: StreamScopeRef, generator: Option<TransactionStreamRef> },
    /// Source-level search that produces a derived table model from waveform data.
    /// Named `source_query` to distinguish from view-level `display_filter`.
    SearchResults { source_query: TableSearchSpec },
    /// Deferred to v2: AnalysisKind and AnalysisParams will define derived metrics
    /// (top 10 spikes, longest idle windows, glitch detection, etc.).
    AnalysisResults { kind: AnalysisKind, params: AnalysisParams },
    Virtual { rows: usize, columns: usize, seed: u64 },
    Custom { key: String, payload: String },
}

/// Serializable view configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableViewConfig {
    pub title: String,
    pub columns: Vec<TableColumnConfig>,
    pub sort: Vec<TableSortSpec>,
    /// View-level filter applied to current table cache (post-source search).
    /// Named `display_filter` to distinguish from model-level `source_query`.
    pub display_filter: TableSearchSpec,
    pub selection_mode: TableSelectionMode,
    /// When true, reduces vertical padding from 4px to 2px and uses smaller font.
    pub dense_rows: bool,
    /// When true, header row stays visible during vertical scroll.
    /// When false, header scrolls with content (rarely desired).
    pub sticky_header: bool,
}

/// Runtime, non-serialized cache handle.
///
/// OnceLock invalidation semantics: OnceLock can only be set once. When cache_key
/// or generation changes, the old TableCacheEntry is dropped and a new one created.
/// OnceLock provides atomic single-write semantics, not mutable updates.
/// This matches the AnalogCacheEntry pattern in analog_signal_cache.rs.
pub struct TableCacheEntry {
    inner: OnceLock<TableCache>,
    pub cache_key: TableCacheKey,
    pub generation: u64,
}

/// Error type for cache build failures.
#[derive(Debug, Clone)]
pub enum TableCacheError {
    /// The referenced model (variable, stream, etc.) was not found.
    ModelNotFound { description: String },
    /// Search pattern compilation failed (e.g., invalid regex).
    InvalidSearch { pattern: String, reason: String },
    /// Underlying waveform/transaction data is not available.
    DataUnavailable,
    /// Cache build was cancelled (e.g., tile closed during build).
    Cancelled,
}
```

### TableModel trait

```rust
pub trait TableModel: Send + Sync {
    fn schema(&self) -> TableSchema;
    fn row_count(&self) -> usize;
    fn row_id_at(&self, index: usize) -> Option<TableRowId>;
    fn cell(&self, row: TableRowId, col: usize) -> TableCell;
    fn sort_key(&self, row: TableRowId, col: usize) -> TableSortKey;
    fn search_text(&self, row: TableRowId) -> String;
    fn on_activate(&self, row: TableRowId) -> TableAction;
}
```

row_id_at defines the model's base order; stable sorting uses this order as a tie-breaker.

### Table view API

```rust
pub fn draw_table_tile(
    state: &mut SystemState,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    msgs: &mut Vec<Message>,
    tile_id: TableTileId,
);
```

### Messages

```rust
pub enum Message {
    AddTableTile { spec: TableModelSpec },
    RemoveTableTile { tile_id: TableTileId },
    SetTableSort { tile_id: TableTileId, sort: Vec<TableSortSpec> },
    SetTableDisplayFilter { tile_id: TableTileId, filter: TableSearchSpec },
    SetTableSelection { tile_id: TableTileId, selection: TableSelection },
    BuildTableCache { tile_id: TableTileId, cache_key: TableCacheKey },
    TableCacheBuilt { tile_id: TableTileId, entry: Arc<TableCacheEntry>, result: Result<TableCache, TableCacheError> },
}
```

### Type glossary (v1)

- TableSchema: ordered list of columns with stable keys and labels, with optional default width/visibility hints. Keys are strings or u64s.
- TableColumnConfig: view layout for a column key (key, width, visibility, resizable). No formatting or alignment overrides in v1. Column drag-to-reorder deferred to v2.
- TableSortSpec: (column key, direction) list used for multi-column sorting.
- TableSearchSpec: { mode, case_sensitive, text }. Empty text means no filter. Used for both source_query (model-level) and display_filter (view-level).
- TableSelectionMode: None | Single | Multi.
- TableSelection: runtime selection state (set of TableRowId + anchor for range selection). Selection persists for filtered-out rows; when filter is cleared, previously selected rows become visible again.
- TableCell: display-ready cell content (string or rich text), optional tooltip. Icons, badges, and progress indicators deferred to v2.
- TableSortKey: sortable value (numeric/string/bytes) used for stable ordering.
- TableCacheKey: { model_key, display_filter, view_sort, generation }. model_key is derived from TableModelSpec via Hash/Eq (SignalChangeList uses variable+field, TransactionTrace uses stream+generator, etc.).
- TableCache: cached row_ids in display order + per-row search text + per-row sort keys.
- TableCacheError: structured error type for cache build failures (ModelNotFound, InvalidSearch, DataUnavailable, Cancelled).
- TableAction: activation result (CursorSet, FocusTransaction, SelectSignal, None).
- Row height: uniform height for all rows to enable virtualization. Variable row heights deferred to v2.

## Implementation details

### File layout

- `libsurfer/src/table/mod.rs` - module root, shared types.
- `libsurfer/src/table/model.rs` - TableModel trait, schema/row types.
- `libsurfer/src/table/view.rs` - egui rendering, interaction handling.
- `libsurfer/src/table/cache.rs` - cache entry, cache build helpers.
- `libsurfer/src/table/sources/` - concrete model implementations.

### Tile integration

- Add `SurferPane::Table(TableTileId)` variant to `tiles.rs`.
- Store table tile configs in UserState, keyed by TableTileId. Tile tree only stores the ID.
- Provide `SurferTileTree::add_table_tile(spec)` to create new tiles and IDs.
- Ensure tab close removes the table tile config and any runtime cache entries.

### Runtime cache flow (analog-inspired)

1. Table view attempts to access a cache entry by key (model identity + display_filter + sort + generation).
2. If the cache is missing or stale (cache_key or generation mismatch), the old TableCacheEntry is dropped and a **new** TableCacheEntry is created. The view renders a loading state and emits Message::BuildTableCache.
3. The SystemState handler starts a worker (perform_work) that builds the row index and any search/sort metadata. Cache building is always off-thread.
4. If a build for the same cache_key is already in-flight, new BuildTableCache requests are ignored to avoid duplicate work.
5. On completion, the worker sends Message::TableCacheBuilt and decrements OUTSTANDING_TRANSACTIONS.
6. The cache entry stores the built TableCache in its OnceLock only if tile_id still exists and the cache_key + generation still match the current view state; otherwise the result is dropped. OnceLock provides atomic single-write semantics. The next frame renders the table normally when a valid cache exists.
7. If cache building fails, a TableCacheError is stored in runtime state for the tile and displayed in the UI until the next successful build or query change.

This mirrors analog cache behavior (AnalogVarState + AnalogCacheEntry + cache_generation).

### Search and sort

- Search uses a TableSearchSpec similar to VariableFilter: mode (contains, regex, fuzzy), case sensitivity, and an input string.
- Regex compilation is cached to avoid per-frame rebuilds; invalid regex yields a TableCacheError::InvalidSearch in the UI.
- Sorting is stable and multi-column (primary, secondary) and uses cached sort keys to avoid repeated string formatting.
- Multi-column sort UI: Click column header to set primary sort (toggles ascending/descending). Shift+click adds or modifies secondary/tertiary sort. Header shows sort indicators (▲/▼) with numbers for multi-column priority (1▲ 2▼). Clicking without Shift resets to single-column sort.
- Search scope split: TableModelSpec::SearchResults uses `source_query` (source-level search built from underlying waveform data by a worker), producing a derived table model. TableViewConfig uses `display_filter` (view-level filter applied to the current table cache). Both can be active: source_query narrows the dataset; display_filter further filters rows in the cache. The UI shows both scopes distinctly (e.g., separate filter badges) to avoid confusion.
- Cache build stores per-row search text and sort keys; TableModel::search_text is called only during cache build, not per frame.

### Selection and activation

- Selection is stored as TableSelection (single/multi). It is keyed by TableRowId to survive sorting.
- Selection persists even for rows filtered out by display_filter. When filter is cleared, previously selected rows become visible again. Selection count UI shows "N selected (M hidden)" when applicable.
- Keyboard navigation:
  - Up/Down: move selection by one row
  - Page Up/Down: move selection by visible page height
  - Home/End: jump to first/last row
  - Ctrl+Home/Ctrl+End: jump to first/last row and scroll
  - Enter: activate selected row
  - Ctrl/Cmd-C: copy selected rows as tab-separated values (visible columns only, no header row). Future: configurable format (CSV, JSON).
  - Type-to-search: typing alphanumeric characters triggers fuzzy jump to matching row (with debounce).
- Activation returns a TableAction, mapped to Messages like CursorSet or FocusTransaction.
- Selection lifecycle: on cache_generation change, selection is cleared because row ids may no longer be valid across reloads. If a model can provide stable ids across reloads in the future, a remap hook can preserve selection.
- Context menus for rows/cells deferred to v2.

### Responsiveness and accessibility

- Use egui_extras::TableBuilder with vscroll(true) and row virtualization.
- Row height is uniform across all rows to enable efficient virtualization. Variable row heights deferred to v2.
- Column widths persist in TableViewConfig; columns can be resizable and auto-sized.
- Rows are drawn with selectable labels for accessibility and keyboard focus.
- Respect theme colors (SurferConfig theme) and ui zoom factor.

### Scroll position behavior

- After sort: scroll to keep the first selected row visible; if none selected, maintain approximate scroll position.
- After filter change: if current scroll position would show empty space beyond content, scroll to keep content visible. If selected row is now filtered out, scroll to top.
- After activation: keep activated row visible (scroll minimally to bring into view if needed).
- Scroll position is runtime-only and not serialized; table opens at top on state load.

### Serialization

- TableTileState (spec + view config) is serialized in UserState.
- Runtime caches and derived data are marked with serde(skip).
- On load, table tiles are recreated and caches rebuilt once data is available.
- Selection state, scroll position, and transient error strings are runtime-only and not serialized.

### Column identity and schema

- TableSchema provides an ordered list of columns with stable keys (string or u64) and labels.
- TableColumnConfig and TableSortSpec reference columns by key, not by index.
- Column order is defined by TableViewConfig.columns; any schema columns not present are appended in schema order.
- Unknown column keys in config/sort are ignored; missing columns are added from the schema with defaults.
- Column widths/visibility do not affect cache_key; only model identity + display_filter + sort + generation do.

### Missing requirements from codebase review

- Use existing time formatting (TimeStringFormatting, TimeUnit) and translator logic to match waveform values.
- Handle waveform reload by invalidating caches via cache_generation (same pattern as analog).
- Avoid storing caches in undo/redo stacks; use manual Clone to drop runtime caches.
- Integrate with Message-based state updates to keep UI changes consistent with existing event flow.
- Support DataContainer::Empty and partially loaded waveforms with clear loading/empty states.
- Wire table actions into command_parser and keyboard_shortcuts (search focus, close tile, copy row).

### Deferred to v2

- AnalysisKind and AnalysisParams: derived metrics like top 10 spikes, longest idle windows, glitch detection.
- Context menus for rows and cells (copy, filter by value, jump to related, etc.).
- Column drag-to-reorder via mouse.
- Variable row heights (requires different virtualization strategy).
- Rich cell content: icons, badges, progress indicators, sparklines.
- Configurable copy format (CSV, JSON, custom delimiter).
- Selection remap hook for preserving selection across reloads when model provides stable cross-generation IDs.

## Testing strategy

- Unit tests for TableModel implementations (row_count, sort_key, search_text correctness).
- Cache tests: build, reuse, invalidate on generation change; ensure no stale results.
- Search tests: contains/regex/fuzzy; invalid regex reports error but does not panic.
- Sorting tests: stable ordering, multi-column sort, Shift+click adds secondary sort, click without Shift resets to single-column.
- Selection tests: selection persists across sorting/filtering (including hidden rows), clears on generation change, and UI shows correct "N selected (M hidden)" counts.
- Serialization tests: TableTileState round-trip in .ron; caches omitted and rebuilt.
- UI snapshot tests: add a virtual table tile and verify rendering and scroll behavior.
- Scroll position tests: verify scroll maintains selected row visibility after sort, scrolls to top when filter hides selected row.
- Keyboard navigation tests: verify Up/Down/Page/Home/End/type-to-search behavior.
- Copy tests: verify tab-separated output format for selected rows.
- Performance tests: large virtual data sets, ensure no per-frame O(n) work and acceptable FPS.

---

## Implementation status

### Completed (Stages 1-11)

The core table infrastructure and first real data model are fully implemented:

**Core types and module structure:**
- `libsurfer/src/table/` module with model.rs, view.rs, cache.rs, and sources/ submodule
- `TableTileId`, `TableRowId`, `TableModelSpec`, `TableViewConfig`, `TableTileState`
- `TableSchema`, `TableColumnConfig`, `TableSortSpec`, `TableSearchSpec`, `TableSearchMode`
- `TableSelectionMode`, `TableSelection`, `TableCell`, `TableSortKey`, `TableAction`
- `TableCacheKey`, `TableCache`, `TableCacheEntry`, `TableCacheError`
- `TableModel` trait with all required methods

**Virtual model (`VirtualTableModel`):**
- Deterministic synthetic data generation for testing (configurable rows, columns, seed)
- Full TableModel trait implementation
- Used for infrastructure validation and snapshot tests

**Async cache system:**
- `TableCacheEntry` with `OnceLock` for atomic cache storage
- Cache builder with filtering (Contains, Exact, Regex, Fuzzy) and multi-column sorting
- `Message::BuildTableCache` and `Message::TableCacheBuilt` handlers
- In-flight deduplication to prevent duplicate cache builds
- Stale result rejection based on cache_key and generation matching

**Tile integration:**
- `SurferPane::Table(TableTileId)` variant in tiles.rs
- `table_tiles: HashMap<TableTileId, TableTileState>` in UserState (serialized)
- `table_runtime: HashMap<TableTileId, TableRuntimeState>` in SystemState (not serialized)
- `SurferTileTree::add_table_tile()` and tile lifecycle management
- Tab close cleanup for config and runtime state

**Table rendering:**
- `egui_extras::TableBuilder` with row virtualization and striped rows
- Header row with column labels from schema
- Loading spinner during cache build, error display on failure
- Dense rows mode (smaller font, reduced padding)
- Theme color integration from SurferConfig

**Sorting:**
- Click-to-sort with ascending/descending toggle
- Shift+click for multi-column sort
- Sort indicators (▲/▼) with priority numbers for multi-column
- `sort_spec_on_click()`, `sort_spec_on_shift_click()`, `sort_indicator()` helpers
- Stable sorting using `row_id_at()` order as tie-breaker

**Display filter (search):**
- Filter bar UI with text input, mode selector (Contains/Exact/Regex/Fuzzy), case toggle
- Live debounced search with cache invalidation
- Row count display: "Showing N of M rows"
- Invalid regex error display
- `fuzzy_match()` for subsequence matching

**Selection:**
- Single and Multi selection modes
- Click, Ctrl+click (toggle), Shift+click (range) selection
- Selection persists across sort/filter changes
- Selection count display: "N selected (M hidden)"
- Selection cleared on waveform reload (generation change)
- `TableSelection` with BTreeSet<TableRowId> and anchor for range selection

**Keyboard navigation:**
- Up/Down, Page Up/Down, Home/End navigation
- Shift+navigation for selection extension
- Enter to activate, Escape to clear selection
- Ctrl+A to select all (Multi mode)
- Ctrl+C to copy selected rows as TSV
- Type-to-search with timeout buffer

**Scroll behavior and polish:**
- Scroll target computation after sort/filter/activation
- Column resize via drag with min width enforcement
- Column visibility toggle via context menu
- Hidden columns bar when columns are hidden
- Generation tracking for waveform reload detection

**SignalChangeList model (`SignalChangeListModel`):**
- Time and Value columns with proper formatting
- `TableModelContext` for accessing WaveData, translators, time formatting
- Lazy row building with `OnceLock<Vec<TransitionRow>>`
- Field path handling for root and subfields
- `on_activate` returns `TableAction::CursorSet(time)`
- Context menu integration in menus.rs for variables
- `table_view` command and keyboard shortcut
- Error handling for missing data/variables

**Test coverage:**
- 200+ unit and integration tests
- Snapshot tests for rendering verification
- All tests passing

---

## Staged implementation plan

### Stage 12: TransactionTrace model

**Goal:** Implement the TransactionTrace table model for FTR transaction traces and wire it into the Surfer UI (context menu + keyboard command) while keeping cache builds async and consistent with existing table infrastructure.

**Prerequisites:** Stages 1-11 complete.

**User experience:**
- Right-click a transaction stream or generator in the transaction hierarchy and choose "Transaction list" to open a new table tile.
- Right-click a focused transaction in the waveform view and choose "Show in table" to open a table filtered to that generator.
- Keyboard command `transaction_table` opens a transaction list for the currently focused transaction's generator.
- Table shows columns for ID, Type, Start, End, Duration, and dynamic attribute columns.
- Activating a row (Enter/double-click) focuses the transaction in the waveform view.
- On waveform reload, the table shows a loading state, cache is rebuilt, and selection clears.

**Deliverables:**

1. **Add `libsurfer/src/table/sources/transaction_trace.rs` with `TransactionTraceModel` implementing `TableModel`:**

   ```rust
   pub struct TransactionTraceModel {
       stream_scope: StreamScopeRef,
       generator_filter: Option<TransactionStreamRef>,
       time_formatter: TimeFormatter,
       rows: OnceLock<TransactionRows>,
       attribute_columns: Vec<String>,
   }

   struct TransactionRows {
       rows: Vec<TransactionRow>,
       index: HashMap<TableRowId, usize>,
   }

   struct TransactionRow {
       tx_id: usize,
       tx_ref: TransactionRef,
       gen_id: usize,
       gen_name: String,
       start_time: BigUint,
       end_time: BigUint,
       duration: BigUint,
       attributes: Vec<(String, String)>,
       start_time_text: String,
       end_time_text: String,
       duration_text: String,
       search_text: String,
   }
   ```

2. **Extend `libsurfer/src/table/sources/mod.rs` to export the model.**

3. **Implement TransactionTraceModel constructor with validation:**
   - Check data availability (waves, transaction container)
   - Validate stream scope exists
   - Validate optional generator filter
   - Defer attribute column discovery to lazy row building

4. **Implement lazy row building (same pattern as SignalChangeListModel):**
   - Collect transactions based on scope (Root, Stream, Generator)
   - Apply generator filter if specified
   - Build TransactionRow with formatted times, search text
   - Sort by start time (base order)
   - Build row index for O(1) lookup

5. **Schema and TableModel trait implementation:**
   - Fixed columns: ID, Type, Start, End, Duration
   - Dynamic attribute columns discovered during row building
   - `row_id_at`: `TableRowId(tx_id as u64)`
   - `sort_key`: Numeric for ID/times, Text for type/attributes
   - `on_activate`: `TableAction::FocusTransaction(tx_ref)`

6. **Wire up `TableModelSpec::TransactionTrace` in factory.**

7. **Add default_view_config for TransactionTrace:**
   - Title based on stream/generator name
   - Default sort: ascending by start time
   - Selection mode: Single

8. **UI integration:**
   - Context menu in transactions.rs: "Transaction list" for stream/generator
   - Context menu for focused transaction: "Show in table"
   - `Message::OpenTransactionTable` message
   - `transaction_table` command in command_parser.rs
   - Handler for `TableAction::FocusTransaction`

**Test Checklist**

Unit tests (TransactionTraceModel methods - 10 tests):
- [ ] `transaction_trace_model_row_count`
- [ ] `transaction_trace_model_row_id_at`
- [ ] `transaction_trace_model_cell_formatting_id`
- [ ] `transaction_trace_model_cell_formatting_type`
- [ ] `transaction_trace_model_cell_formatting_times`
- [ ] `transaction_trace_model_cell_formatting_duration`
- [ ] `transaction_trace_model_cell_formatting_attributes`
- [ ] `transaction_trace_model_sort_key_numeric_for_times`
- [ ] `transaction_trace_model_search_text_includes_all_fields`
- [ ] `transaction_trace_model_on_activate_returns_focus_transaction`

Unit tests (Error handling - 4 tests):
- [ ] `transaction_trace_model_no_wave_data_returns_data_unavailable`
- [ ] `transaction_trace_model_no_transaction_data_returns_data_unavailable`
- [ ] `transaction_trace_model_stream_not_found_returns_model_not_found`
- [ ] `transaction_trace_model_generator_not_found_returns_model_not_found`

Unit tests (Scope filtering - 5 tests):
- [ ] `transaction_trace_model_root_scope_includes_all_transactions`
- [ ] `transaction_trace_model_stream_scope_filters_to_stream`
- [ ] `transaction_trace_model_generator_scope_filters_to_generator`
- [ ] `transaction_trace_model_generator_filter_applies_within_stream`
- [ ] `transaction_trace_model_empty_scope_returns_empty`

Unit tests (Dynamic schema - 3 tests):
- [ ] `transaction_trace_model_schema_includes_fixed_columns`
- [ ] `transaction_trace_model_schema_includes_attribute_columns`
- [ ] `transaction_trace_model_schema_deduplicates_attribute_names`

Unit tests (Duration calculation - 2 tests):
- [ ] `transaction_trace_duration_calculated_correctly`
- [ ] `transaction_trace_duration_zero_when_start_equals_end`

Integration tests (Message handling - 6 tests):
- [ ] `open_transaction_table_creates_tile_for_stream`
- [ ] `open_transaction_table_creates_tile_for_generator`
- [ ] `transaction_table_command_opens_table_for_focused_transaction`
- [ ] `transaction_table_activate_focuses_transaction`
- [ ] `transaction_table_tile_removed_on_close`
- [ ] `transaction_table_cache_rebuilds_on_reload`

Snapshot tests (visual verification - 3 tests):
- [ ] `table_transaction_trace_renders_columns` - Table shows ID/Type/Start/End/Duration columns
- [ ] `table_transaction_trace_with_attributes` - Attribute columns appear in table
- [ ] `table_transaction_trace_empty_stream` - Empty state for stream with no transactions

**Total new tests: 33** (24 unit + 6 integration + 3 snapshot)

**Implementation notes:**
- `TransactionTraceModel` uses `OnceLock<TransactionRows>` for lazy row building (same pattern as SignalChangeListModel)
- Constructor validates stream/generator existence but defers row building to first access
- Attribute columns are discovered during row building by scanning all transactions
- Time formatting uses `TimeFormatter` from context (same as SignalChangeListModel)
- Row ID is `TableRowId(tx_id as u64)` - transaction IDs are unique within FTR data
- `TableAction::FocusTransaction` already exists in model.rs
- Existing `Message::FocusTransaction(Option<TransactionRef>, Option<Transaction>)` handles activation
- Test data: Use `examples/` FTR files or create minimal test FTR data

---

### Stage 13: SearchResults model

**Goal:** Implement source-level search producing a derived table.

**Prerequisites:** Stages 1-11 complete.

**Deliverables:**
- Implement `SearchResultsModel` in `sources/search_results.rs`:
  - Constructor: takes `TableSearchSpec` (source_query), searches across signals.
  - Schema: columns for "Signal", "Time", "Value", "Context".
  - Each row: one occurrence of search pattern in waveform data.
  - `row_id_at(index)`: composite ID from signal + timestamp.
  - `on_activate(row)`: return `TableAction::CursorSet` + highlight signal.
- Wire up `TableModelSpec::SearchResults` in factory.
- Search runs in worker thread (can be slow for large waveforms).
- Show progress indicator for long searches.

**Acceptance tests:**
- [ ] Unit test: Search finds correct occurrences in test data.
- [ ] Unit test: Results include signal name and context.
- [ ] Integration test: Activating row jumps to time and highlights signal.
- [ ] Integration test: Long search shows progress, can be cancelled.
- [ ] UI snapshot test: SearchResults renders for sample search.

---

### Stage 14: Custom model support

**Goal:** Enable external/plugin models via Custom variant.

**Prerequisites:** Stages 1-11 complete.

**Deliverables:**
- Define `CustomModelRegistry` for registering model factories by key.
- Wire up `TableModelSpec::Custom` to look up factory by key.
- Document API for creating custom models.
- Example: WASM-based custom table model.

**Acceptance tests:**
- [ ] Unit test: Custom model factory registration works.
- [ ] Unit test: Unknown custom key returns `ModelNotFound` error.
- [ ] Integration test: Custom model renders in table tile.

---

### Stage 15 (v2): AnalysisResults model

**Goal:** Implement derived analysis metrics (deferred to v2).

**Prerequisites:** Stages 1-11 complete, analysis framework designed.

**Deliverables:**
- Define `AnalysisKind` enum: `TopSpikes`, `LongestIdle`, `GlitchDetection`, etc.
- Define `AnalysisParams` for configuring each analysis type.
- Implement analysis workers that produce table rows.
- Wire up `TableModelSpec::AnalysisResults` in factory.

**Acceptance tests:**
- [ ] To be defined when v2 analysis framework is designed.

---

### Implementation order summary

```
Completed:
  Stages 1-11: Core infrastructure + SignalChangeList model
     ↓
Next up (can be parallelized):
  Stage 12 (TransactionTrace)  ← current priority
  Stage 13 (SearchResults)
  Stage 14 (Custom)
     ↓
v2 (deferred):
  Stage 15 (AnalysisResults)
```

---

## Changelog

- **Stages 1-11 complete:** Core table infrastructure with 200+ tests
  - Types, traits, cache system, tile integration
  - Virtual model for testing
  - Full sorting, filtering, selection, keyboard navigation
  - Scroll behavior, column resize/visibility
  - SignalChangeList model with UI integration
- **Stage 12 designed:** TransactionTrace model with detailed implementation plan
  - Follows Stage 11 patterns (TableModelContext, lazy row building, error handling)
  - Dynamic attribute columns, scope filtering, duration calculation
  - 33 planned tests (24 unit + 6 integration + 3 snapshot)
- **Stages 13-15:** Pending implementation
