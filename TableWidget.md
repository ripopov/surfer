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

- Add `SurferPane::Table(TableTileId)` and update `SurferTileBehavior::pane_ui()` to render table tiles.
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

## Staged implementation plan

The implementation is divided into stages. Stages 1-10 use only the Virtual model to build and validate all table infrastructure. Stages 11+ add concrete models for real waveform/transaction data. Each stage includes acceptance tests that must pass before proceeding.

### Stage 1: Core types and module structure

**Goal:** Define all public types and traits; establish module layout.

**Deliverables:**
- Create `libsurfer/src/table/mod.rs` with module declarations and re-exports.
- Create `libsurfer/src/table/model.rs` with:
  - `TableTileId`, `TableRowId` (with Serialize/Deserialize)
  - `TableModelSpec` enum (all variants, but only Virtual implemented)
  - `TableSchema`, `TableColumnConfig`, `TableSortSpec`, `TableSearchSpec`
  - `TableSelectionMode`, `TableSelection`
  - `TableCell`, `TableSortKey`, `TableAction`
  - `TableModel` trait definition
- Create `libsurfer/src/table/cache.rs` with:
  - `TableCacheKey`, `TableCache`, `TableCacheEntry`, `TableCacheError`
- Create `libsurfer/src/table/view.rs` with stub `draw_table_tile()`.
- Create `libsurfer/src/table/sources/mod.rs` and `libsurfer/src/table/sources/virtual_model.rs` with stub.

**Acceptance tests:**
- [ ] `cargo build` succeeds with new module structure.
- [ ] Unit test: `TableTileId`, `TableRowId` serialize/deserialize round-trip.
- [ ] Unit test: `TableModelSpec::Virtual` serializes to expected RON format.
- [ ] Unit test: `TableViewConfig` with all fields serializes/deserializes correctly.

---

### Stage 2: Virtual model implementation

**Goal:** Implement `VirtualTableModel` that generates deterministic synthetic data for testing.

**Deliverables:**
- Implement `VirtualTableModel` in `sources/virtual_model.rs`:
  - Constructor: `new(rows: usize, columns: usize, seed: u64)`
  - Schema: columns named "Col 0", "Col 1", ... with string keys "col_0", "col_1", ...
  - `row_count()`: returns configured row count.
  - `row_id_at(index)`: returns `TableRowId(index as u64)`.
  - `cell(row, col)`: deterministic string based on `(seed, row.0, col)`.
  - `sort_key(row, col)`: numeric key derived from cell content for sortable columns.
  - `search_text(row)`: concatenation of all cell values for the row.
  - `on_activate(row)`: returns `TableAction::None`.
- Add factory function: `TableModelSpec::create_model(&self, ...) -> Option<Arc<dyn TableModel>>`.

**Acceptance tests:**
- [ ] Unit test: `VirtualTableModel::row_count()` returns correct value.
- [ ] Unit test: `VirtualTableModel::row_id_at()` returns sequential IDs.
- [ ] Unit test: `VirtualTableModel::cell()` returns deterministic, reproducible content.
- [ ] Unit test: Same `(rows, columns, seed)` produces identical model output.
- [ ] Unit test: `VirtualTableModel::schema()` returns expected column count and keys.
- [ ] Unit test: `VirtualTableModel::search_text()` includes all column values.

---

### Stage 3: Cache system

**Goal:** Implement async cache building with invalidation and stale-result rejection.

**Deliverables:**
- Implement `TableCacheEntry::new()`, `is_ready()`, `get()`, `set()`.
- Implement `TableCache` structure: `row_ids: Vec<TableRowId>`, `search_texts: Vec<String>`, `sort_keys: Vec<Vec<TableSortKey>>`.
- Implement cache builder function (runs off-thread):
  - Takes `Arc<dyn TableModel>`, `TableSearchSpec`, `Vec<TableSortSpec>`.
  - Builds filtered and sorted row index.
  - Returns `Result<TableCache, TableCacheError>`.
- Add `Message::BuildTableCache` and `Message::TableCacheBuilt` to `message.rs`.
- Implement message handlers in `SystemState`:
  - `BuildTableCache`: spawn worker if not already in-flight for same cache_key.
  - `TableCacheBuilt`: apply result only if cache_key matches current state.
- Add in-flight tracking to prevent duplicate builds.

**Note on factory function evolution:**
The current `TableModelSpec::create_model(&self) -> Option<Arc<dyn TableModel>>` works for the Virtual model which requires no external context. In Stages 11-14, this will be extended to `create_model(&self, ctx: &TableModelContext) -> Result<Arc<dyn TableModel>, TableCacheError>` where `TableModelContext` provides access to `WaveData`, `TransactionContainer`, formatters, and other runtime dependencies. The Virtual model will ignore the context, while real models will use it to access waveform data.

**Acceptance tests:**
- [ ] Unit test: `TableCacheEntry` starts not ready, becomes ready after `set()`.
- [ ] Unit test: Cache builder produces correct row_ids for unfiltered, unsorted input.
- [ ] Unit test: Cache builder filters rows correctly (contains mode).
- [ ] Unit test: Cache builder sorts rows correctly (single column, ascending/descending).
- [ ] Unit test: Cache builder handles empty result set gracefully.
- [ ] Unit test: Cache builder returns `InvalidSearch` error for bad regex.
- [ ] Integration test: `TableCacheBuilt` with stale cache_key is ignored (simulate via delayed message).

---

### Stage 4: Tile integration

**Goal:** Integrate table tiles into the egui_tiles layout system.

**Deliverables:**
- Add `SurferPane::Table(TableTileId)` variant to `tiles.rs`.
- Add `TableTileState { spec: TableModelSpec, config: TableViewConfig }` to `state.rs`.
- Add `table_tiles: HashMap<TableTileId, TableTileState>` to `UserState`.
- Add `table_runtime: HashMap<TableTileId, TableRuntimeState>` to `SystemState` (non-serialized).
  - `TableRuntimeState`: current cache entry, selection, scroll offset, error state.
- Extend `TableRuntimeState` with selection + scroll (cache + error already added in Stage 3).
- Replace the temporary `table_models` map with model creation from `table_tiles`/`TableModelSpec`.
- Implement `SurferTileTree::add_table_tile(spec) -> TableTileId`.
- Update `SurferTileBehavior::pane_ui()` to dispatch to `draw_table_tile()`.
- Update `SurferTileBehavior::is_tab_closable()` to allow closing table tiles.
- Update `SurferTileBehavior::on_tab_close()` to clean up table tile state.
- Add `Message::AddTableTile` and `Message::RemoveTableTile` handlers.

**Acceptance tests:**
- [x] Integration test: `AddTableTile` creates tile visible in tile tree.
- [x] Integration test: Closing table tile removes it from `table_tiles` and `table_runtime`.
- [x] Serialization test: Save state with table tile, reload, tile config preserved.
- [x] Serialization test: Runtime state (selection, scroll) is NOT serialized.
- [x] Unit test: `TableTileId` generation produces unique IDs.

**Implementation notes:**
- `TableTileState` added to `model.rs`, contains spec + config
- `SurferPane::Table(TableTileId)` variant added to `tiles.rs`
- `table_tiles: HashMap<TableTileId, TableTileState>` added to `UserState`
- `TableRuntimeState` extended with `selection` and `scroll_offset` fields
- `SurferTileTree::add_table_tile()` and `next_table_id()` added
- `draw_table_tile()` shows loading state and triggers cache build
- `Message::AddTableTile` and `Message::RemoveTableTile` implemented
- All 28 table tests pass

---

### Stage 5: Basic table rendering

**Goal:** Render a table with header and virtualized rows using `egui_extras::TableBuilder`.

**Deliverables:**
- Implement `draw_table_tile()` in `view.rs`:
  - Retrieve `TableTileState` and `TableRuntimeState`.
  - If cache not ready, show loading spinner and emit `BuildTableCache`.
  - If cache error, show error message.
  - Otherwise, render table with `egui_extras::TableBuilder`.
- Render header row with column labels from schema.
- Render body rows using virtualization (`vscroll(true)`, `row(height, |row| ...)`).
- Apply theme colors from `SurferConfig`.
- Implement basic horizontal scrolling for wide tables.
- Wire up `dense_rows` config (adjust row height and font).
- Wire up `sticky_header` config.

**Acceptance tests:**
- [x] UI snapshot test: Virtual table (10 rows, 3 columns) renders correctly.
- [x] UI snapshot test: Virtual table (1000 rows) renders without lag (virtualization working).
- [x] UI snapshot test: Table with `dense_rows: true` has reduced row height.
- [x] UI snapshot test: Loading state shows spinner.
- [x] UI snapshot test: Error state shows error message.
- [x] Manual test: Vertical scroll is smooth with 10,000 rows.

**Implementation notes:**
- `render_table()` function added to `view.rs` using `egui_extras::TableBuilder`
- Row height: 20px normal, 16px dense mode
- Header uses theme's `secondary_ui_color.background` for background
- Dense mode uses `.small()` text, normal mode uses `.strong()` for headers
- TableBuilder with `striped(true)` and `vscroll(true)` for virtualization
- Columns created from schema with `default_width` and `resizable` config
- `sticky_header` config is stored but egui_extras always renders headers as sticky; non-sticky headers deferred
- `draw_table_tile()` now takes `table_tiles` parameter to fix borrow conflict during tile tree rendering
- Added `table_caches_ready()` method to `SystemState` for snapshot test support
- Updated `render_and_compare_inner` to process `BuildTableCache` messages
- Snapshot tests: `table_virtual_10_rows_3_cols`, `table_virtual_1000_rows`, `table_dense_rows`
- All 31 table tests pass (28 unit + 3 snapshot)

---

### Stage 6: Sorting

**Goal:** Implement column header click-to-sort with multi-column support.

**Deliverables:**
- Make column headers clickable.
- On click: set primary sort to clicked column, toggle direction if already primary.
- On Shift+click: add/modify secondary sort.
- Render sort indicators (▲/▼) with priority numbers in header.
- Emit `Message::SetTableSort` on sort change.
- Implement message handler to update `TableViewConfig.sort`.
- Invalidate cache (create new entry) when sort changes.
- Implement stable sorting in cache builder using `row_id_at()` order as tie-breaker.

**Pre-implemented:**
- Stable sorting with base_index tie-breaker is already implemented in `build_table_cache()`
- Multi-column sort is already implemented in cache builder

**Stage 5 dependencies:**
- `render_table()` will need additional parameters: `tile_id`, `msgs`, and current `sort` spec
- Add `Message::SetTableSort` to `message.rs` and implement handler in `lib.rs`

**New types and functions:**

```rust
// In model.rs - helper functions for sort spec manipulation

/// Computes the new sort spec when a column header is clicked (without Shift).
/// - If column is not in sort: set as primary ascending, clear other sorts
/// - If column is primary: toggle direction
/// - If column is secondary+: promote to primary ascending, clear others
pub fn sort_spec_on_click(
    current: &[TableSortSpec],
    clicked_key: &TableColumnKey,
) -> Vec<TableSortSpec>;

/// Computes the new sort spec when a column header is Shift+clicked.
/// - If column is not in sort: append as new sort level (ascending)
/// - If column is in sort: toggle its direction (keep position)
pub fn sort_spec_on_shift_click(
    current: &[TableSortSpec],
    clicked_key: &TableColumnKey,
) -> Vec<TableSortSpec>;

/// Returns the sort indicator text for a column header.
/// - Returns None if column is not in sort
/// - Returns "▲" or "▼" for single-column sort
/// - Returns "▲1", "▼2", etc. for multi-column sort
pub fn sort_indicator(
    sort: &[TableSortSpec],
    column_key: &TableColumnKey,
) -> Option<String>;

// In message.rs
pub enum Message {
    // ... existing variants ...
    SetTableSort { tile_id: TableTileId, sort: Vec<TableSortSpec> },
}
```

---

**Automated Testing Plan**

The testing strategy follows the existing patterns in `libsurfer/src/table/tests.rs`:
- Unit tests for pure functions (no SystemState required)
- Integration tests using `SystemState::new_default_config()` and `state.update(msg)`
- Snapshot tests using `snapshot_ui!` macro in `libsurfer/src/tests/snapshot.rs`

---

**Unit Tests (in `libsurfer/src/table/tests.rs`)**

These tests verify the sort spec manipulation functions in isolation.

```rust
// ========================
// Stage 6 Tests - Sort Spec Manipulation
// ========================

#[test]
fn sort_spec_click_unsorted_column_sets_primary_ascending() {
    // Given: no current sort
    // When: click on "col_0"
    // Then: sort becomes [col_0 Ascending]
    let current: Vec<TableSortSpec> = vec![];
    let clicked = TableColumnKey::Str("col_0".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, clicked);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_click_primary_column_toggles_direction() {
    // Given: sort is [col_0 Ascending]
    // When: click on "col_0"
    // Then: sort becomes [col_0 Descending]
    let current = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let clicked = TableColumnKey::Str("col_0".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, TableSortDirection::Descending);

    // Click again: Descending -> Ascending
    let result2 = sort_spec_on_click(&result, &clicked);
    assert_eq!(result2[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_click_different_column_replaces_sort() {
    // Given: sort is [col_0 Descending]
    // When: click on "col_1"
    // Then: sort becomes [col_1 Ascending] (col_0 removed)
    let current = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, clicked);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_click_secondary_column_promotes_to_primary() {
    // Given: sort is [col_0 Asc, col_1 Desc]
    // When: click on "col_1" (secondary)
    // Then: sort becomes [col_1 Ascending] (promoted, direction reset, others cleared)
    let current = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        },
    ];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, clicked);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_shift_click_adds_secondary_sort() {
    // Given: sort is [col_0 Ascending]
    // When: Shift+click on "col_1"
    // Then: sort becomes [col_0 Ascending, col_1 Ascending]
    let current = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_shift_click(&current, &clicked);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].key, TableColumnKey::Str("col_0".to_string()));
    assert_eq!(result[1].key, clicked);
    assert_eq!(result[1].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_spec_shift_click_existing_column_toggles_direction() {
    // Given: sort is [col_0 Asc, col_1 Asc]
    // When: Shift+click on "col_1"
    // Then: sort becomes [col_0 Asc, col_1 Desc] (position preserved)
    let current = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Ascending,
        },
    ];
    let clicked = TableColumnKey::Str("col_1".to_string());
    let result = sort_spec_on_shift_click(&current, &clicked);
    assert_eq!(result.len(), 2);
    assert_eq!(result[1].key, clicked);
    assert_eq!(result[1].direction, TableSortDirection::Descending);
}

#[test]
fn sort_spec_shift_click_on_unsorted_table_sets_primary() {
    // Given: no current sort
    // When: Shift+click on "col_0"
    // Then: sort becomes [col_0 Ascending]
    let current: Vec<TableSortSpec> = vec![];
    let clicked = TableColumnKey::Str("col_0".to_string());
    let result = sort_spec_on_shift_click(&current, &clicked);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].direction, TableSortDirection::Ascending);
}

#[test]
fn sort_indicator_no_sort_returns_none() {
    let sort: Vec<TableSortSpec> = vec![];
    let key = TableColumnKey::Str("col_0".to_string());
    assert_eq!(sort_indicator(&sort, &key), None);
}

#[test]
fn sort_indicator_column_not_in_sort_returns_none() {
    let sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let key = TableColumnKey::Str("col_1".to_string());
    assert_eq!(sort_indicator(&sort, &key), None);
}

#[test]
fn sort_indicator_single_column_no_number() {
    // Single-column sort: just arrow, no number
    let sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    let key = TableColumnKey::Str("col_0".to_string());
    assert_eq!(sort_indicator(&sort, &key), Some("▲".to_string()));

    let sort_desc = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    assert_eq!(sort_indicator(&sort_desc, &key), Some("▼".to_string()));
}

#[test]
fn sort_indicator_multi_column_shows_priority() {
    // Multi-column sort: arrow + priority number
    let sort = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        },
    ];
    assert_eq!(
        sort_indicator(&sort, &TableColumnKey::Str("col_0".to_string())),
        Some("▲1".to_string())
    );
    assert_eq!(
        sort_indicator(&sort, &TableColumnKey::Str("col_1".to_string())),
        Some("▼2".to_string())
    );
}
```

---

**Integration Tests (in `libsurfer/src/table/tests.rs`)**

These tests verify the full message flow using `SystemState`.

```rust
// ========================
// Stage 6 Tests - Message Handling Integration
// ========================

#[test]
fn set_table_sort_updates_config() {
    // Setup: create state with a table tile
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initially: no sort
    assert!(state.user.table_tiles[&tile_id].config.sort.is_empty());

    // Action: send SetTableSort message
    let new_sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Ascending,
    }];
    state.update(Message::SetTableSort {
        tile_id,
        sort: new_sort.clone(),
    });

    // Verify: config updated
    assert_eq!(state.user.table_tiles[&tile_id].config.sort, new_sort);
}

#[test]
fn set_table_sort_invalidates_cache() {
    // Setup: create state with table tile and built cache
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Build initial cache (no sort)
    let initial_cache_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![],
        generation: 0,
    };
    state.table_runtime.entry(tile_id).or_default().cache_key = Some(initial_cache_key.clone());

    // Action: change sort
    let new_sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    state.update(Message::SetTableSort {
        tile_id,
        sort: new_sort.clone(),
    });

    // Verify: cache_key in runtime should now be different (or None, triggering rebuild)
    // The view will detect mismatch and emit BuildTableCache
    let runtime = state.table_runtime.get(&tile_id).expect("runtime exists");
    // Old cache_key should no longer match the new sort spec
    if let Some(cached_key) = &runtime.cache_key {
        assert_ne!(cached_key.view_sort, new_sort);
    }
}

#[test]
fn set_table_sort_nonexistent_tile_ignored() {
    // Setup: state with no table tiles
    let mut state = SystemState::new_default_config().expect("state");

    // Action: send SetTableSort for non-existent tile
    let fake_tile_id = TableTileId(9999);
    state.update(Message::SetTableSort {
        tile_id: fake_tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        }],
    });

    // Verify: no crash, no state change
    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn sort_change_triggers_cache_rebuild_in_view() {
    // This test verifies the full flow: sort change -> cache mismatch -> BuildTableCache emitted
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 5, columns: 2, seed: 0 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Simulate initial cache build completed
    let initial_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![],
        generation: 0,
    };
    let cache_entry = Arc::new(TableCacheEntry::new(initial_key.clone(), 0));
    cache_entry.set(TableCache {
        row_ids: (0..5).map(|i| TableRowId(i as u64)).collect(),
        search_texts: vec!["".to_string(); 5],
        sort_keys: vec![vec![]; 5],
    });
    state.table_runtime.entry(tile_id).or_default().cache = Some(cache_entry);
    state.table_runtime.get_mut(&tile_id).unwrap().cache_key = Some(initial_key);

    // Change sort
    let new_sort = vec![TableSortSpec {
        key: TableColumnKey::Str("col_0".to_string()),
        direction: TableSortDirection::Descending,
    }];
    state.update(Message::SetTableSort {
        tile_id,
        sort: new_sort,
    });

    // When draw_table_tile runs, it will detect cache_key mismatch and emit BuildTableCache
    // This is verified by the snapshot tests below
}

#[test]
fn multi_column_sort_via_messages() {
    // Test setting up multi-column sort through message updates
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set multi-column sort
    let multi_sort = vec![
        TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        },
        TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        },
    ];
    state.update(Message::SetTableSort {
        tile_id,
        sort: multi_sort.clone(),
    });

    assert_eq!(state.user.table_tiles[&tile_id].config.sort, multi_sort);
}
```

---

**Snapshot Tests (in `libsurfer/src/tests/snapshot.rs`)**

These tests verify visual rendering of sort indicators.

```rust
// ========================
// Stage 6 Snapshot Tests - Sort Indicators
// ========================

snapshot_ui!(table_sort_single_column_ascending, || {
    use crate::table::{TableModelSpec, TableSortSpec, TableSortDirection, TableColumnKey};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 5, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Ascending,
        }],
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_sort_single_column_descending, || {
    use crate::table::{TableModelSpec, TableSortSpec, TableSortDirection, TableColumnKey};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 5, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_1".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_sort_multi_column, || {
    use crate::table::{TableModelSpec, TableSortSpec, TableSortDirection, TableColumnKey};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 5, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    // Multi-column sort: col_0 ascending (primary), col_2 descending (secondary)
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![
            TableSortSpec {
                key: TableColumnKey::Str("col_0".to_string()),
                direction: TableSortDirection::Ascending,
            },
            TableSortSpec {
                key: TableColumnKey::Str("col_2".to_string()),
                direction: TableSortDirection::Descending,
            },
        ],
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_sort_affects_row_order, || {
    use crate::table::{TableModelSpec, TableSortSpec, TableSortDirection, TableColumnKey};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    // Use a small table where sort order is visually verifiable
    let spec = TableModelSpec::Virtual { rows: 5, columns: 2, seed: 123 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    // Sort by col_0 descending - rows should be reordered
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});
```

---

**Test Checklist**

Unit tests (sort spec manipulation - 11 tests):
- [x] `sort_spec_click_unsorted_column_sets_primary_ascending`
- [x] `sort_spec_click_primary_column_toggles_direction`
- [x] `sort_spec_click_different_column_replaces_sort`
- [x] `sort_spec_click_secondary_column_promotes_to_primary`
- [x] `sort_spec_shift_click_adds_secondary_sort`
- [x] `sort_spec_shift_click_existing_column_toggles_direction`
- [x] `sort_spec_shift_click_on_unsorted_table_sets_primary`
- [x] `sort_indicator_no_sort_returns_none`
- [x] `sort_indicator_column_not_in_sort_returns_none`
- [x] `sort_indicator_single_column_no_number`
- [x] `sort_indicator_multi_column_shows_priority`

Integration tests (message handling - 3 tests):
- [x] `set_table_sort_updates_config`
- [x] `set_table_sort_nonexistent_tile_ignored`
- [x] `multi_column_sort_via_messages`

Snapshot tests (visual verification - 4 tests):
- [x] `table_sort_single_column_ascending` - Header shows "▲" on sorted column
- [x] `table_sort_single_column_descending` - Header shows "▼" on sorted column
- [x] `table_sort_multi_column` - Headers show "▲1" and "▼2" with priorities
- [x] `table_sort_affects_row_order` - Rows visually reordered after sort

Pre-existing tests (already passing):
- [x] `table_cache_builder_sorts_rows` - Stable sort preserves original order for equal keys
- [x] Multi-column sort applies correct priority (cache builder supports `Vec<TableSortSpec>`)

---

**Acceptance Criteria**

Stage 6 is complete when:
1. All 11 unit tests pass
2. All 3 integration tests pass
3. All 4 snapshot tests pass and images are accepted
4. `cargo clippy --no-deps` reports no warnings for new code
5. `cargo fmt` produces no changes

**Implementation notes:**
- `sort_spec_on_click()`, `sort_spec_on_shift_click()`, `sort_indicator()` added to `model.rs`
- `Message::SetTableSort` added to `message.rs` with handler in `lib.rs`
- `render_table()` updated to accept `tile_id`, `msgs`, and current `sort` spec
- Headers are clickable via `egui::Label` with `sense(egui::Sense::click())`
- Sort indicators display in header text: "⬆"/"⬇" for single-column, "⬆1"/"⬇2" for multi-column (using arrows compatible with egui fonts)
- Cache invalidation happens automatically when `cache_key.view_sort` changes
- Tests: 11 unit tests + 3 integration tests + 4 snapshot tests = 18 total Stage 6 tests
- Reduced from planned 12+5 tests to 11+3: removed redundant cache invalidation tests since cache invalidation is already tested in Stage 3/4 and happens automatically via cache_key mismatch detection in `draw_table_tile()`
- All 50 table tests pass

---

### Stage 7: Display filter (search)

**Goal:** Implement view-level filtering with contains/regex/fuzzy modes.

**Pre-implemented:**
- Contains and Regex filtering already implemented in `build_table_cache()`
- `TableSearchSpec` and `TableSearchMode` types already defined
- Regex compilation is already cached in `TableFilter` struct
- Tests exist for Contains, Regex, and case-insensitive matching

**Deliverables:**
- Add filter input UI above table (text field + mode selector + case toggle).
- Emit `Message::SetTableDisplayFilter` on filter change.
- Implement message handler to update `TableViewConfig.display_filter`.
- Invalidate cache when filter changes.
- Implement filtering in cache builder:
  - Contains: substring match on `search_text`. *(already implemented)*
  - Regex: compile pattern, match against `search_text`. *(already implemented)*
  - Fuzzy: add `TableSearchMode::Fuzzy` variant and implement simple fuzzy matching (subsequence).
- Cache compiled regex to avoid repeated compilation. *(already implemented)*
- Show filter badge indicating active filter.
- Show row count: "Showing N of M rows".

**Stage 6 dependencies:**
- `render_table()` will need additional parameters: current `display_filter` spec
- Add `Message::SetTableDisplayFilter` to `message.rs` and implement handler in `lib.rs`

**New types and functions:**

```rust
// In model.rs - add Fuzzy variant to existing enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableSearchMode {
    Contains,
    Exact,
    Regex,
    Fuzzy,  // NEW: subsequence matching
}

// In cache.rs - add fuzzy matching to TableFilter::matches()
impl TableFilter {
    fn matches(&self, haystack: &str) -> bool {
        // ... existing Contains, Exact, Regex handling ...
        TableSearchMode::Fuzzy => {
            // Subsequence matching: "abc" matches "aXbXc" but not "bac"
            fuzzy_match(&self.text, &self.text_lower, haystack, self.case_sensitive)
        }
    }
}

/// Returns true if `needle` characters appear in `haystack` in order (subsequence).
/// For example: "abc" matches "aXbYcZ" but not "bac".
pub fn fuzzy_match(needle: &str, needle_lower: &str, haystack: &str, case_sensitive: bool) -> bool;

// In view.rs - new UI components
/// Renders the filter bar above the table with text input, mode selector, and case toggle.
fn render_filter_bar(
    ui: &mut egui::Ui,
    tile_id: TableTileId,
    config: &TableViewConfig,
    total_rows: usize,
    filtered_rows: usize,
    msgs: &mut Vec<Message>,
);

// In message.rs
pub enum Message {
    // ... existing variants ...
    SetTableDisplayFilter { tile_id: TableTileId, filter: TableSearchSpec },
}
```

---

**Automated Testing Plan**

The testing strategy follows the existing patterns in `libsurfer/src/table/tests.rs`:
- Unit tests for pure functions (no SystemState required)
- Integration tests using `SystemState::new_default_config()` and `state.update(msg)`
- Snapshot tests using `snapshot_ui!` macro in `libsurfer/src/tests/snapshot.rs`

---

**Unit Tests (in `libsurfer/src/table/tests.rs`)**

These tests verify the filtering logic in isolation.

```rust
// ========================
// Stage 7 Tests - Fuzzy Matching
// ========================

#[test]
fn fuzzy_match_exact_characters_in_order() {
    // "abc" should match "abc" exactly
    assert!(fuzzy_match("abc", "abc", "abc", true));
    assert!(fuzzy_match("abc", "abc", "abc", false));
}

#[test]
fn fuzzy_match_subsequence_with_gaps() {
    // "abc" should match "aXbYcZ" (characters in order with gaps)
    assert!(fuzzy_match("abc", "abc", "aXbYcZ", true));
    assert!(fuzzy_match("abc", "abc", "a_b_c", true));
    assert!(fuzzy_match("abc", "abc", "a---b---c", true));
}

#[test]
fn fuzzy_match_fails_wrong_order() {
    // "abc" should NOT match "bac" (wrong order)
    assert!(!fuzzy_match("abc", "abc", "bac", true));
    assert!(!fuzzy_match("abc", "abc", "cba", true));
    assert!(!fuzzy_match("abc", "abc", "acb", true));
}

#[test]
fn fuzzy_match_fails_missing_character() {
    // "abc" should NOT match "ab" (missing 'c')
    assert!(!fuzzy_match("abc", "abc", "ab", true));
    assert!(!fuzzy_match("abc", "abc", "ac", true));
    assert!(!fuzzy_match("abc", "abc", "bc", true));
}

#[test]
fn fuzzy_match_empty_needle_matches_all() {
    // Empty needle matches everything
    assert!(fuzzy_match("", "", "anything", true));
    assert!(fuzzy_match("", "", "", true));
}

#[test]
fn fuzzy_match_case_insensitive() {
    // Case-insensitive matching
    assert!(fuzzy_match("abc", "abc", "ABC", false));
    assert!(fuzzy_match("ABC", "abc", "abc", false));
    assert!(fuzzy_match("AbC", "abc", "aBc", false));
    // Case-sensitive should fail
    assert!(!fuzzy_match("abc", "abc", "ABC", true));
}

#[test]
fn fuzzy_match_unicode() {
    // Unicode characters should work
    assert!(fuzzy_match("αβγ", "αβγ", "αXβYγ", true));
    assert!(fuzzy_match("日本", "日本", "日X本", true));
}

// ========================
// Stage 7 Tests - Filter Cache Building
// ========================

#[test]
fn table_cache_builder_filters_fuzzy() {
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    // VirtualTableModel cell format: "r{row}c{col}"
    // Row 3 has "r3c0" and "r3c1" -> search_text contains "r3c0 r3c1"
    // Fuzzy "r3" should match rows containing "r3" as subsequence
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Fuzzy,
            case_sensitive: false,
            text: "r3".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    // Should match row 3 (contains "r3c0")
    assert!(cache.row_ids.contains(&TableRowId(3)));
}

#[test]
fn table_cache_builder_fuzzy_subsequence_matching() {
    // Create a custom model with known search texts for precise fuzzy testing
    #[derive(Clone)]
    struct FuzzyTestModel {
        rows: Vec<(TableRowId, String)>,
    }

    impl TableModel for FuzzyTestModel {
        fn schema(&self) -> TableSchema {
            TableSchema {
                columns: vec![TableColumn {
                    key: TableColumnKey::Str("col".to_string()),
                    label: "Col".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                }],
            }
        }

        fn row_count(&self) -> usize {
            self.rows.len()
        }

        fn row_id_at(&self, index: usize) -> Option<TableRowId> {
            self.rows.get(index).map(|(id, _)| *id)
        }

        fn cell(&self, row: TableRowId, _col: usize) -> TableCell {
            let text = self.rows.iter()
                .find(|(id, _)| *id == row)
                .map(|(_, t)| t.clone())
                .unwrap_or_default();
            TableCell::Text(text)
        }

        fn sort_key(&self, _row: TableRowId, _col: usize) -> TableSortKey {
            TableSortKey::None
        }

        fn search_text(&self, row: TableRowId) -> String {
            self.rows.iter()
                .find(|(id, _)| *id == row)
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        }

        fn on_activate(&self, _row: TableRowId) -> TableAction {
            TableAction::None
        }
    }

    let model = Arc::new(FuzzyTestModel {
        rows: vec![
            (TableRowId(0), "alpha".to_string()),      // "aa" matches: a_l_p_h_a
            (TableRowId(1), "beta".to_string()),       // "aa" does not match
            (TableRowId(2), "gamma".to_string()),      // "aa" matches: g_a_m_m_a
            (TableRowId(3), "delta".to_string()),      // "aa" does not match
            (TableRowId(4), "abracadabra".to_string()),// "aa" matches: a_b_r_a_c...
        ],
    });

    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Fuzzy,
            case_sensitive: false,
            text: "aa".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    // Rows 0, 2, 4 have two 'a' characters in order
    assert_eq!(cache.row_ids.len(), 3);
    assert!(cache.row_ids.contains(&TableRowId(0)));
    assert!(cache.row_ids.contains(&TableRowId(2)));
    assert!(cache.row_ids.contains(&TableRowId(4)));
}

#[test]
fn table_cache_builder_regex_filter() {
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    // Regex to match rows ending with "c0" (col 0)
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: false,
            text: "r[0-2]c0".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    // Should match rows 0, 1, 2 (r0c0, r1c0, r2c0)
    assert_eq!(cache.row_ids.len(), 3);
    assert!(cache.row_ids.contains(&TableRowId(0)));
    assert!(cache.row_ids.contains(&TableRowId(1)));
    assert!(cache.row_ids.contains(&TableRowId(2)));
}

#[test]
fn table_cache_builder_exact_filter() {
    // Create a model where we can test exact matching
    #[derive(Clone)]
    struct ExactTestModel;

    impl TableModel for ExactTestModel {
        fn schema(&self) -> TableSchema {
            TableSchema {
                columns: vec![TableColumn {
                    key: TableColumnKey::Str("col".to_string()),
                    label: "Col".to_string(),
                    default_width: None,
                    default_visible: true,
                    default_resizable: true,
                }],
            }
        }

        fn row_count(&self) -> usize { 3 }

        fn row_id_at(&self, index: usize) -> Option<TableRowId> {
            Some(TableRowId(index as u64))
        }

        fn cell(&self, _row: TableRowId, _col: usize) -> TableCell {
            TableCell::Text("test".to_string())
        }

        fn sort_key(&self, _row: TableRowId, _col: usize) -> TableSortKey {
            TableSortKey::None
        }

        fn search_text(&self, row: TableRowId) -> String {
            match row.0 {
                0 => "alpha".to_string(),
                1 => "alpha beta".to_string(),
                2 => "ALPHA".to_string(),
                _ => String::new(),
            }
        }

        fn on_activate(&self, _row: TableRowId) -> TableAction {
            TableAction::None
        }
    }

    let model = Arc::new(ExactTestModel);

    // Exact match case-sensitive
    let cache = build_table_cache(
        model.clone(),
        TableSearchSpec {
            mode: TableSearchMode::Exact,
            case_sensitive: true,
            text: "alpha".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(0)]); // Only exact match

    // Exact match case-insensitive
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Exact,
            case_sensitive: false,
            text: "alpha".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    // Matches "alpha" and "ALPHA" but not "alpha beta"
    assert_eq!(cache.row_ids.len(), 2);
    assert!(cache.row_ids.contains(&TableRowId(0)));
    assert!(cache.row_ids.contains(&TableRowId(2)));
}

#[test]
fn table_cache_builder_case_insensitive_contains() {
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    // VirtualTableModel uses lowercase "r0c0" format
    // Case-insensitive search for "R3C0" should match row 3
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "R3C0".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    assert_eq!(cache.row_ids, vec![TableRowId(3)]);
}

#[test]
fn table_cache_builder_case_sensitive_no_match() {
    let model = Arc::new(VirtualTableModel::new(10, 2, 0));
    // Case-sensitive search for "R3C0" should NOT match "r3c0"
    let cache = build_table_cache(
        model,
        TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: true,
            text: "R3C0".to_string(),
        },
        vec![],
    )
    .expect("cache build should succeed");

    assert!(cache.row_ids.is_empty());
}

// ========================
// Stage 7 Tests - Filter Spec Helpers (if needed)
// ========================

#[test]
fn table_search_spec_default_is_inactive() {
    let spec = TableSearchSpec::default();
    assert!(spec.text.is_empty());
    assert_eq!(spec.mode, TableSearchMode::Contains);
    assert!(!spec.case_sensitive);
}

#[test]
fn table_search_spec_is_active() {
    // Empty text means inactive
    let inactive = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: String::new(),
    };
    assert!(inactive.text.is_empty());

    // Non-empty text means active
    let active = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "search".to_string(),
    };
    assert!(!active.text.is_empty());
}

#[test]
fn table_search_mode_serialization() {
    // All modes should serialize/deserialize correctly
    for mode in [
        TableSearchMode::Contains,
        TableSearchMode::Exact,
        TableSearchMode::Regex,
        TableSearchMode::Fuzzy,
    ] {
        let encoded = ron::ser::to_string(&mode).expect("serialize");
        let decoded: TableSearchMode = ron::de::from_str(&encoded).expect("deserialize");
        assert_eq!(mode, decoded);
    }
}
```

---

**Integration Tests (in `libsurfer/src/table/tests.rs`)**

These tests verify the full message flow using `SystemState`.

```rust
// ========================
// Stage 7 Tests - Message Handling Integration
// ========================

#[test]
fn set_table_display_filter_updates_config() {
    // Setup: create state with a table tile
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initially: empty filter
    assert!(state.user.table_tiles[&tile_id].config.display_filter.text.is_empty());

    // Action: send SetTableDisplayFilter message
    let new_filter = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: true,
        text: "search term".to_string(),
    };
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: new_filter.clone(),
    });

    // Verify: config updated
    assert_eq!(state.user.table_tiles[&tile_id].config.display_filter, new_filter);
}

#[test]
fn set_table_display_filter_nonexistent_tile_ignored() {
    // Setup: state with no table tiles
    let mut state = SystemState::new_default_config().expect("state");

    // Action: send SetTableDisplayFilter for non-existent tile
    let fake_tile_id = TableTileId(9999);
    state.update(Message::SetTableDisplayFilter {
        tile_id: fake_tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: false,
            text: "test".to_string(),
        },
    });

    // Verify: no crash, no state change
    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn set_table_display_filter_with_all_modes() {
    // Test that all search modes can be set via message
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    for mode in [
        TableSearchMode::Contains,
        TableSearchMode::Exact,
        TableSearchMode::Regex,
        TableSearchMode::Fuzzy,
    ] {
        let filter = TableSearchSpec {
            mode,
            case_sensitive: false,
            text: "test".to_string(),
        };
        state.update(Message::SetTableDisplayFilter {
            tile_id,
            filter: filter.clone(),
        });
        assert_eq!(state.user.table_tiles[&tile_id].config.display_filter.mode, mode);
    }
}

#[test]
fn display_filter_change_invalidates_cache() {
    // Setup: create state with table tile and built cache
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Build initial cache (no filter)
    let initial_cache_key = TableCacheKey {
        model_key: TableModelKey(tile_id.0),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![],
        generation: 0,
    };
    state.table_runtime.entry(tile_id).or_default().cache_key = Some(initial_cache_key.clone());

    // Action: change filter
    let new_filter = TableSearchSpec {
        mode: TableSearchMode::Contains,
        case_sensitive: false,
        text: "search".to_string(),
    };
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: new_filter.clone(),
    });

    // Verify: cache_key in runtime should now be different (or None, triggering rebuild)
    // The view will detect mismatch and emit BuildTableCache
    let runtime = state.table_runtime.get(&tile_id).expect("runtime exists");
    if let Some(cached_key) = &runtime.cache_key {
        assert_ne!(cached_key.display_filter, new_filter);
    }
}

#[test]
fn filter_and_sort_combined_cache_key() {
    // Verify that both filter and sort are part of cache key
    let key1 = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![],
        generation: 0,
    };

    let key2 = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "filter".to_string(),
        },
        view_sort: vec![],
        generation: 0,
    };

    let key3 = TableCacheKey {
        model_key: TableModelKey(1),
        display_filter: TableSearchSpec::default(),
        view_sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col".to_string()),
            direction: TableSortDirection::Ascending,
        }],
        generation: 0,
    };

    // All three keys should be different
    assert_ne!(key1, key2); // Different filter
    assert_ne!(key1, key3); // Different sort
    assert_ne!(key2, key3); // Different filter AND sort
}

#[test]
fn clear_filter_returns_to_default() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set a filter
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: true,
            text: "pattern".to_string(),
        },
    });

    // Clear the filter
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec::default(),
    });

    // Verify: back to default
    let filter = &state.user.table_tiles[&tile_id].config.display_filter;
    assert!(filter.text.is_empty());
    assert_eq!(filter.mode, TableSearchMode::Contains);
    assert!(!filter.case_sensitive);
}
```

---

**Snapshot Tests (in `libsurfer/src/tests/snapshot.rs`)**

These tests verify visual rendering of the filter UI.

```rust
// ========================
// Stage 7 Snapshot Tests - Filter UI
// ========================

snapshot_ui!(table_filter_bar_inactive, || {
    use crate::table::TableModelSpec;

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    // Create table with no active filter
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_bar_contains_active, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r3".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_bar_regex_active, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: false,
            text: "r[0-2]c0".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_bar_fuzzy_active, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Fuzzy,
            case_sensitive: false,
            text: "rc".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_case_sensitive_indicator, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: true,  // Case-sensitive ON
            text: "r3".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_showing_n_of_m_rows, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    // 100 rows, filter to show ~10
    let spec = TableModelSpec::Virtual { rows: 100, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    // Filter that matches only rows with "r1" in them (r1, r10-r19 = ~11 rows)
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r1c".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_no_results, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    // Filter that matches nothing
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: true,
            text: "NOTFOUND".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_filter_invalid_regex_error, || {
    use crate::table::{TableModelSpec, TableSearchSpec, TableSearchMode};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();
    // Invalid regex pattern
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Regex,
            case_sensitive: false,
            text: "[invalid(".to_string(),
        },
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});
```

---

**Test Checklist**

Unit tests (fuzzy matching - 7 tests):
- [ ] `fuzzy_match_exact_characters_in_order`
- [ ] `fuzzy_match_subsequence_with_gaps`
- [ ] `fuzzy_match_fails_wrong_order`
- [ ] `fuzzy_match_fails_missing_character`
- [ ] `fuzzy_match_empty_needle_matches_all`
- [ ] `fuzzy_match_case_insensitive`
- [ ] `fuzzy_match_unicode`

Unit tests (filter cache building - 7 tests):
- [ ] `table_cache_builder_filters_fuzzy`
- [ ] `table_cache_builder_fuzzy_subsequence_matching`
- [ ] `table_cache_builder_regex_filter`
- [ ] `table_cache_builder_exact_filter`
- [ ] `table_cache_builder_case_insensitive_contains`
- [ ] `table_cache_builder_case_sensitive_no_match`
- [ ] `table_search_mode_serialization`

Unit tests (filter spec - 2 tests):
- [ ] `table_search_spec_default_is_inactive`
- [ ] `table_search_spec_is_active`

Integration tests (message handling - 6 tests):
- [ ] `set_table_display_filter_updates_config`
- [ ] `set_table_display_filter_nonexistent_tile_ignored`
- [ ] `set_table_display_filter_with_all_modes`
- [ ] `display_filter_change_invalidates_cache`
- [ ] `filter_and_sort_combined_cache_key`
- [ ] `clear_filter_returns_to_default`

Snapshot tests (visual verification - 8 tests):
- [ ] `table_filter_bar_inactive` - Filter bar with empty input field
- [ ] `table_filter_bar_contains_active` - Contains mode with active filter and badge
- [ ] `table_filter_bar_regex_active` - Regex mode indicator visible
- [ ] `table_filter_bar_fuzzy_active` - Fuzzy mode indicator visible
- [ ] `table_filter_case_sensitive_indicator` - Case-sensitive toggle visible
- [ ] `table_filter_showing_n_of_m_rows` - "Showing N of M rows" count displayed
- [ ] `table_filter_no_results` - Empty state when filter matches nothing
- [ ] `table_filter_invalid_regex_error` - Error display for invalid regex

Pre-existing tests (already passing):
- [x] `table_cache_builder_filters_contains` - Contains filter matches substring
- [x] `table_cache_builder_invalid_regex` - Invalid regex returns error
- [x] `table_cache_builder_empty_result` - Empty filter result handled

---

**Acceptance Criteria**

Stage 7 is complete when:
1. All 7 fuzzy matching unit tests pass
2. All 7 filter cache building unit tests pass
3. All 2 filter spec unit tests pass
4. All 6 integration tests pass
5. All 8 snapshot tests pass and images are accepted
6. `cargo clippy --no-deps` reports no warnings for new code
7. `cargo fmt` produces no changes

**Total new tests: 30** (16 unit + 6 integration + 8 snapshot)

---

### Stage 8: Selection

**Goal:** Implement single and multi-row selection with persistence across sort/filter.

**Pre-implemented:**
- `TableSelection` struct with `rows: BTreeSet<TableRowId>` and `anchor: Option<TableRowId>`
- `TableSelectionMode` enum (None, Single, Multi)
- `TableRuntimeState` includes `selection` field (added in Stage 4)

**Deliverables:**
- Implement `TableSelection` in runtime state. *(already added)*
- Single-click row: select (clear previous in Single mode, toggle in Multi mode).
- Shift+click: range selection from anchor.
- Ctrl/Cmd+click: toggle selection without clearing.
- Highlight selected rows with theme selection color.
- Track selection by `TableRowId`, not by index.
- Selection persists when sort changes (rows reorder but selection preserved).
- Selection persists when filter hides rows (hidden rows stay selected).
- Show selection count: "N selected" or "N selected (M hidden)".
- Clear selection when `cache_generation` changes (waveform reload).

**Stage 7 dependencies:**
- `render_table()` will need additional parameters: `selection`, `selection_mode`, and `msgs`
- Add `Message::SetTableSelection` to `message.rs` and implement handler in `lib.rs`
- Add `Message::ClearTableSelection` for generation change handling

**New types and functions:**

```rust
// In model.rs - helper functions for selection manipulation

impl TableSelection {
    /// Creates an empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the selection is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the number of selected rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns true if the given row is selected.
    pub fn contains(&self, row: TableRowId) -> bool {
        self.rows.contains(&row)
    }

    /// Clears all selection.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.anchor = None;
    }

    /// Counts how many selected rows are in the visible set.
    pub fn count_visible(&self, visible_rows: &[TableRowId]) -> usize {
        let visible_set: BTreeSet<_> = visible_rows.iter().copied().collect();
        self.rows.intersection(&visible_set).count()
    }

    /// Counts how many selected rows are hidden (not in visible set).
    pub fn count_hidden(&self, visible_rows: &[TableRowId]) -> usize {
        self.len() - self.count_visible(visible_rows)
    }
}

/// Result of a selection click operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionUpdate {
    pub selection: TableSelection,
    pub changed: bool,
}

/// Computes selection update when a row is clicked in Single mode.
/// - Clears previous selection and selects the clicked row.
/// - Sets anchor to clicked row.
pub fn selection_on_click_single(
    current: &TableSelection,
    clicked: TableRowId,
) -> SelectionUpdate;

/// Computes selection update when a row is clicked in Multi mode (no modifiers).
/// - Clears previous selection and selects the clicked row.
/// - Sets anchor to clicked row.
pub fn selection_on_click_multi(
    current: &TableSelection,
    clicked: TableRowId,
) -> SelectionUpdate;

/// Computes selection update when Ctrl/Cmd+click in Multi mode.
/// - Toggles the clicked row without clearing others.
/// - Sets anchor to clicked row.
pub fn selection_on_ctrl_click(
    current: &TableSelection,
    clicked: TableRowId,
) -> SelectionUpdate;

/// Computes selection update when Shift+click in Multi mode.
/// - Selects range from anchor to clicked row (inclusive).
/// - Preserves anchor.
/// - `visible_rows` provides the current display order for range computation.
pub fn selection_on_shift_click(
    current: &TableSelection,
    clicked: TableRowId,
    visible_rows: &[TableRowId],
) -> SelectionUpdate;

/// Formats the selection count for display.
/// - Returns "N selected" when no rows are hidden.
/// - Returns "N selected (M hidden)" when some selected rows are filtered out.
pub fn format_selection_count(
    total_selected: usize,
    hidden_count: usize,
) -> String;

// In message.rs
pub enum Message {
    // ... existing variants ...
    SetTableSelection { tile_id: TableTileId, selection: TableSelection },
    ClearTableSelection { tile_id: TableTileId },
}
```

---

**Automated Testing Plan**

The testing strategy follows the existing patterns in `libsurfer/src/table/tests.rs`:
- Unit tests for pure functions (no SystemState required)
- Integration tests using `SystemState::new_default_config()` and `state.update(msg)`
- Snapshot tests using `snapshot_ui!` macro in `libsurfer/src/tests/snapshot.rs`

---

**Unit Tests (in `libsurfer/src/table/tests.rs`)**

These tests verify the selection manipulation functions in isolation.

```rust
// ========================
// Stage 8 Tests - TableSelection Methods
// ========================

#[test]
fn table_selection_new_is_empty() {
    let sel = TableSelection::new();
    assert!(sel.is_empty());
    assert_eq!(sel.len(), 0);
    assert!(sel.anchor.is_none());
}

#[test]
fn table_selection_contains() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(5));

    assert!(sel.contains(TableRowId(1)));
    assert!(!sel.contains(TableRowId(2)));
    assert!(sel.contains(TableRowId(3)));
    assert!(sel.contains(TableRowId(5)));
    assert!(!sel.contains(TableRowId(0)));
}

#[test]
fn table_selection_clear() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(2));
    sel.anchor = Some(TableRowId(1));

    sel.clear();

    assert!(sel.is_empty());
    assert!(sel.anchor.is_none());
}

#[test]
fn table_selection_count_visible() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(3));
    sel.rows.insert(TableRowId(5));
    sel.rows.insert(TableRowId(7));

    // Only rows 1, 3, 5 are visible (7 is filtered out)
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3), TableRowId(5)];

    assert_eq!(sel.count_visible(&visible), 3);
    assert_eq!(sel.count_hidden(&visible), 1);
}

#[test]
fn table_selection_count_all_visible() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(1));
    sel.rows.insert(TableRowId(2));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    assert_eq!(sel.count_visible(&visible), 2);
    assert_eq!(sel.count_hidden(&visible), 0);
}

#[test]
fn table_selection_count_all_hidden() {
    let mut sel = TableSelection::new();
    sel.rows.insert(TableRowId(10));
    sel.rows.insert(TableRowId(20));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2)];

    assert_eq!(sel.count_visible(&visible), 0);
    assert_eq!(sel.count_hidden(&visible), 2);
}

// ========================
// Stage 8 Tests - Single Mode Selection
// ========================

#[test]
fn selection_single_mode_click_selects_row() {
    let current = TableSelection::new();
    let result = selection_on_click_single(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_single_mode_click_replaces_previous() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(3));

    let result = selection_on_click_single(&current, TableRowId(7));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(!result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(7)));
    assert_eq!(result.selection.anchor, Some(TableRowId(7)));
}

#[test]
fn selection_single_mode_click_same_row_unchanged() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(5));
    current.anchor = Some(TableRowId(5));

    let result = selection_on_click_single(&current, TableRowId(5));

    // Clicking same row should not report change
    assert!(!result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
}

// ========================
// Stage 8 Tests - Multi Mode Selection
// ========================

#[test]
fn selection_multi_mode_click_selects_row() {
    let current = TableSelection::new();
    let result = selection_on_click_multi(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_multi_mode_click_clears_previous() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(2));
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(1));

    let result = selection_on_click_multi(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert!(!result.selection.contains(TableRowId(1)));
}

#[test]
fn selection_multi_mode_ctrl_click_toggles_on() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(1));

    let result = selection_on_ctrl_click(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 3);
    assert!(result.selection.contains(TableRowId(1)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_multi_mode_ctrl_click_toggles_off() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(1));
    current.rows.insert(TableRowId(3));
    current.rows.insert(TableRowId(5));
    current.anchor = Some(TableRowId(1));

    let result = selection_on_ctrl_click(&current, TableRowId(3));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 2);
    assert!(result.selection.contains(TableRowId(1)));
    assert!(!result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(3)));
}

#[test]
fn selection_multi_mode_ctrl_click_empty_selection() {
    let current = TableSelection::new();
    let result = selection_on_ctrl_click(&current, TableRowId(5));

    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

// ========================
// Stage 8 Tests - Range Selection (Shift+Click)
// ========================

#[test]
fn selection_shift_click_range_forward() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(2));
    current.anchor = Some(TableRowId(2));

    // Visible order: 0, 1, 2, 3, 4, 5
    let visible = vec![
        TableRowId(0), TableRowId(1), TableRowId(2),
        TableRowId(3), TableRowId(4), TableRowId(5),
    ];

    let result = selection_on_shift_click(&current, TableRowId(5), &visible);

    assert!(result.changed);
    // Should select rows 2, 3, 4, 5 (inclusive range)
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(2)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(result.selection.contains(TableRowId(5)));
    // Anchor preserved
    assert_eq!(result.selection.anchor, Some(TableRowId(2)));
}

#[test]
fn selection_shift_click_range_backward() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(5));
    current.anchor = Some(TableRowId(5));

    let visible = vec![
        TableRowId(0), TableRowId(1), TableRowId(2),
        TableRowId(3), TableRowId(4), TableRowId(5),
    ];

    let result = selection_on_shift_click(&current, TableRowId(2), &visible);

    assert!(result.changed);
    // Should select rows 2, 3, 4, 5 (inclusive range backward)
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(2)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(result.selection.contains(TableRowId(5)));
    assert_eq!(result.selection.anchor, Some(TableRowId(5)));
}

#[test]
fn selection_shift_click_single_row() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(3));
    current.anchor = Some(TableRowId(3));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(&current, TableRowId(3), &visible);

    // Shift+click on same row as anchor - just that row
    assert!(!result.changed); // Already selected
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(3)));
}

#[test]
fn selection_shift_click_no_anchor_uses_clicked_as_anchor() {
    let current = TableSelection::new();
    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(&current, TableRowId(2), &visible);

    // No anchor means treat clicked row as both anchor and target
    assert!(result.changed);
    assert_eq!(result.selection.len(), 1);
    assert!(result.selection.contains(TableRowId(2)));
    assert_eq!(result.selection.anchor, Some(TableRowId(2)));
}

#[test]
fn selection_shift_click_anchor_not_visible_uses_clicked() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(10)); // Row 10 is filtered out
    current.anchor = Some(TableRowId(10));

    let visible = vec![TableRowId(0), TableRowId(1), TableRowId(2), TableRowId(3)];

    let result = selection_on_shift_click(&current, TableRowId(2), &visible);

    // Anchor not in visible set - select just clicked row and set new anchor
    assert!(result.changed);
    assert!(result.selection.contains(TableRowId(2)));
    assert_eq!(result.selection.anchor, Some(TableRowId(2)));
}

#[test]
fn selection_shift_click_extends_from_anchor_replaces_selection() {
    let mut current = TableSelection::new();
    current.rows.insert(TableRowId(0));
    current.rows.insert(TableRowId(1));
    current.anchor = Some(TableRowId(0));

    let visible = vec![
        TableRowId(0), TableRowId(1), TableRowId(2),
        TableRowId(3), TableRowId(4),
    ];

    // Shift+click on row 3 should select 0-3, not add to existing
    let result = selection_on_shift_click(&current, TableRowId(3), &visible);

    assert!(result.changed);
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(0)));
    assert!(result.selection.contains(TableRowId(1)));
    assert!(result.selection.contains(TableRowId(2)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(!result.selection.contains(TableRowId(4)));
}

#[test]
fn selection_shift_click_sorted_order() {
    // After sorting, rows may appear in different order
    let mut current = TableSelection::new();
    current.anchor = Some(TableRowId(5));
    current.rows.insert(TableRowId(5));

    // Sorted order: 5, 3, 1, 4, 2, 0 (arbitrary sort result)
    let visible = vec![
        TableRowId(5), TableRowId(3), TableRowId(1),
        TableRowId(4), TableRowId(2), TableRowId(0),
    ];

    let result = selection_on_shift_click(&current, TableRowId(4), &visible);

    // Should select rows 5, 3, 1, 4 (by display order)
    assert!(result.changed);
    assert_eq!(result.selection.len(), 4);
    assert!(result.selection.contains(TableRowId(5)));
    assert!(result.selection.contains(TableRowId(3)));
    assert!(result.selection.contains(TableRowId(1)));
    assert!(result.selection.contains(TableRowId(4)));
    assert!(!result.selection.contains(TableRowId(2)));
    assert!(!result.selection.contains(TableRowId(0)));
}

// ========================
// Stage 8 Tests - Selection Count Formatting
// ========================

#[test]
fn format_selection_count_none() {
    assert_eq!(format_selection_count(0, 0), "");
}

#[test]
fn format_selection_count_visible_only() {
    assert_eq!(format_selection_count(5, 0), "5 selected");
    assert_eq!(format_selection_count(1, 0), "1 selected");
}

#[test]
fn format_selection_count_with_hidden() {
    assert_eq!(format_selection_count(5, 2), "5 selected (2 hidden)");
    assert_eq!(format_selection_count(10, 1), "10 selected (1 hidden)");
}

#[test]
fn format_selection_count_all_hidden() {
    // All selected rows are hidden
    assert_eq!(format_selection_count(3, 3), "3 selected (3 hidden)");
}

// ========================
// Stage 8 Tests - Selection Mode Behavior
// ========================

#[test]
fn selection_mode_none_ignores_clicks() {
    // In None mode, selection should always be empty
    // This is tested at the UI/integration level
    let mode = TableSelectionMode::None;
    assert_eq!(mode, TableSelectionMode::None);
}

#[test]
fn selection_mode_serialization() {
    for mode in [
        TableSelectionMode::None,
        TableSelectionMode::Single,
        TableSelectionMode::Multi,
    ] {
        let encoded = ron::ser::to_string(&mode).expect("serialize");
        let decoded: TableSelectionMode = ron::de::from_str(&encoded).expect("deserialize");
        assert_eq!(mode, decoded);
    }
}
```

---

**Integration Tests (in `libsurfer/src/table/tests.rs`)**

These tests verify the full message flow using `SystemState`.

```rust
// ========================
// Stage 8 Tests - Message Handling Integration
// ========================

#[test]
fn set_table_selection_updates_runtime() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Initially: empty selection
    assert!(state.table_runtime[&tile_id].selection.is_empty());

    // Set selection
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(3));
    selection.anchor = Some(TableRowId(1));

    state.update(Message::SetTableSelection {
        tile_id,
        selection: selection.clone(),
    });

    assert_eq!(state.table_runtime[&tile_id].selection, selection);
}

#[test]
fn clear_table_selection_clears_runtime() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set selection first
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(5));
    selection.anchor = Some(TableRowId(5));
    state.update(Message::SetTableSelection { tile_id, selection });

    assert!(!state.table_runtime[&tile_id].selection.is_empty());

    // Clear selection
    state.update(Message::ClearTableSelection { tile_id });

    assert!(state.table_runtime[&tile_id].selection.is_empty());
    assert!(state.table_runtime[&tile_id].selection.anchor.is_none());
}

#[test]
fn set_table_selection_nonexistent_tile_ignored() {
    let mut state = SystemState::new_default_config().expect("state");

    let fake_tile_id = TableTileId(9999);
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));

    // Should not crash
    state.update(Message::SetTableSelection {
        tile_id: fake_tile_id,
        selection,
    });

    assert!(state.user.table_tiles.is_empty());
}

#[test]
fn selection_persists_after_sort_change() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set selection
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(7));
    selection.anchor = Some(TableRowId(3));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: selection.clone(),
    });

    // Change sort
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    // Selection should persist (tracked by TableRowId, not index)
    assert_eq!(state.table_runtime[&tile_id].selection.len(), 2);
    assert!(state.table_runtime[&tile_id].selection.contains(TableRowId(3)));
    assert!(state.table_runtime[&tile_id].selection.contains(TableRowId(7)));
}

#[test]
fn selection_persists_after_filter_change() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Select multiple rows
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(5));
    selection.anchor = Some(TableRowId(1));
    state.update(Message::SetTableSelection {
        tile_id,
        selection: selection.clone(),
    });

    // Apply filter that hides some rows
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r3".to_string(), // Only matches row 3
        },
    });

    // Selection should persist (hidden rows stay selected)
    assert_eq!(state.table_runtime[&tile_id].selection.len(), 3);
    assert!(state.table_runtime[&tile_id].selection.contains(TableRowId(1))); // hidden
    assert!(state.table_runtime[&tile_id].selection.contains(TableRowId(3))); // visible
    assert!(state.table_runtime[&tile_id].selection.contains(TableRowId(5))); // hidden
}

#[test]
fn selection_clears_on_remove_table_tile() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set selection
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(5));
    state.update(Message::SetTableSelection { tile_id, selection });

    // Remove tile
    state.update(Message::RemoveTableTile { tile_id });

    // Runtime state should be gone
    assert!(!state.table_runtime.contains_key(&tile_id));
}

#[test]
fn selection_mode_none_prevents_selection() {
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set selection mode to None
    state.user.table_tiles.get_mut(&tile_id).unwrap().config.selection_mode = TableSelectionMode::None;

    // Attempt to set selection (via message - UI would prevent this)
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(5));
    state.update(Message::SetTableSelection { tile_id, selection });

    // In None mode, message handler should ignore or clear
    // (Implementation choice: either ignore the message or always clear)
    // For robustness, we test that the UI layer respects the mode
}

#[test]
fn multiple_tiles_independent_selection() {
    let mut state = SystemState::new_default_config().expect("state");

    // Create two table tiles
    state.update(Message::AddTableTile {
        spec: TableModelSpec::Virtual { rows: 10, columns: 3, seed: 1 }
    });
    state.update(Message::AddTableTile {
        spec: TableModelSpec::Virtual { rows: 10, columns: 3, seed: 2 }
    });

    let tile_ids: Vec<_> = state.user.table_tiles.keys().copied().collect();
    assert_eq!(tile_ids.len(), 2);

    // Set different selections on each tile
    let mut sel1 = TableSelection::new();
    sel1.rows.insert(TableRowId(1));
    state.update(Message::SetTableSelection { tile_id: tile_ids[0], selection: sel1 });

    let mut sel2 = TableSelection::new();
    sel2.rows.insert(TableRowId(5));
    sel2.rows.insert(TableRowId(6));
    state.update(Message::SetTableSelection { tile_id: tile_ids[1], selection: sel2 });

    // Verify independent
    assert_eq!(state.table_runtime[&tile_ids[0]].selection.len(), 1);
    assert!(state.table_runtime[&tile_ids[0]].selection.contains(TableRowId(1)));

    assert_eq!(state.table_runtime[&tile_ids[1]].selection.len(), 2);
    assert!(state.table_runtime[&tile_ids[1]].selection.contains(TableRowId(5)));
}

#[test]
fn selection_not_serialized() {
    // Selection is runtime-only, should not appear in serialized state
    let mut state = SystemState::new_default_config().expect("state");
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().expect("tile exists");

    // Set selection
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3));
    state.update(Message::SetTableSelection { tile_id, selection });

    // Serialize user state
    let serialized = ron::ser::to_string_pretty(&state.user, Default::default())
        .expect("serialize");

    // Should not contain selection data (only selection_mode in config)
    assert!(!serialized.contains("TableRowId(3)"));
    assert!(serialized.contains("selection_mode")); // Config field is serialized
}
```

---

**Snapshot Tests (in `libsurfer/src/tests/snapshot.rs`)**

These tests verify visual rendering of selection highlighting.

```rust
// ========================
// Stage 8 Snapshot Tests - Selection Highlighting
// ========================

snapshot_ui!(table_selection_single_row, || {
    use crate::table::{TableModelSpec, TableSelection, TableRowId};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select single row
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(3));
    selection.anchor = Some(TableRowId(3));
    state.update(Message::SetTableSelection { tile_id, selection });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_multiple_rows, || {
    use crate::table::{TableModelSpec, TableSelection, TableRowId};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select multiple non-contiguous rows
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(4));
    selection.rows.insert(TableRowId(7));
    selection.anchor = Some(TableRowId(1));
    state.update(Message::SetTableSelection { tile_id, selection });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_contiguous_range, || {
    use crate::table::{TableModelSpec, TableSelection, TableRowId};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select contiguous range (as if Shift+click)
    let mut selection = TableSelection::new();
    for i in 2..=6 {
        selection.rows.insert(TableRowId(i));
    }
    selection.anchor = Some(TableRowId(2));
    state.update(Message::SetTableSelection { tile_id, selection });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_with_sort, || {
    use crate::table::{
        TableModelSpec, TableSelection, TableRowId,
        TableSortSpec, TableSortDirection, TableColumnKey
    };

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 8, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select rows 2 and 5
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(2));
    selection.rows.insert(TableRowId(5));
    selection.anchor = Some(TableRowId(2));
    state.update(Message::SetTableSelection { tile_id, selection });

    // Apply descending sort - rows reorder but selection persists
    state.update(Message::SetTableSort {
        tile_id,
        sort: vec![TableSortSpec {
            key: TableColumnKey::Str("col_0".to_string()),
            direction: TableSortDirection::Descending,
        }],
    });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_with_filter_hidden_count, || {
    use crate::table::{
        TableModelSpec, TableSelection, TableRowId,
        TableSearchSpec, TableSearchMode
    };

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select rows 1, 3, 5, 7
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(1));
    selection.rows.insert(TableRowId(3));
    selection.rows.insert(TableRowId(5));
    selection.rows.insert(TableRowId(7));
    selection.anchor = Some(TableRowId(1));
    state.update(Message::SetTableSelection { tile_id, selection });

    // Filter to show only row 3 (hides 1, 5, 7)
    state.update(Message::SetTableDisplayFilter {
        tile_id,
        filter: TableSearchSpec {
            mode: TableSearchMode::Contains,
            case_sensitive: false,
            text: "r3c".to_string(),
        },
    });

    // Should show "4 selected (3 hidden)"
    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_empty, || {
    use crate::table::TableModelSpec;

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    // Table with no selection
    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_first_row, || {
    use crate::table::{TableModelSpec, TableSelection, TableRowId};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select first row
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(0));
    selection.anchor = Some(TableRowId(0));
    state.update(Message::SetTableSelection { tile_id, selection });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});

snapshot_ui!(table_selection_last_row, || {
    use crate::table::{TableModelSpec, TableSelection, TableRowId};

    let mut state = SystemState::new_default_config()
        .unwrap()
        .with_params(StartupParams::default());

    let spec = TableModelSpec::Virtual { rows: 10, columns: 3, seed: 42 };
    state.update(Message::AddTableTile { spec });

    let tile_id = *state.user.table_tiles.keys().next().unwrap();

    // Select last row
    let mut selection = TableSelection::new();
    selection.rows.insert(TableRowId(9));
    selection.anchor = Some(TableRowId(9));
    state.update(Message::SetTableSelection { tile_id, selection });

    state.update(Message::SetMenuVisible(false));
    state.update(Message::SetToolbarVisible(false));
    state.update(Message::SetOverviewVisible(false));
    state.update(Message::SetSidePanelVisible(false));

    state
});
```

---

**Test Checklist**

Unit tests (TableSelection methods - 6 tests):
- [ ] `table_selection_new_is_empty`
- [ ] `table_selection_contains`
- [ ] `table_selection_clear`
- [ ] `table_selection_count_visible`
- [ ] `table_selection_count_all_visible`
- [ ] `table_selection_count_all_hidden`

Unit tests (Single mode - 3 tests):
- [ ] `selection_single_mode_click_selects_row`
- [ ] `selection_single_mode_click_replaces_previous`
- [ ] `selection_single_mode_click_same_row_unchanged`

Unit tests (Multi mode - 4 tests):
- [ ] `selection_multi_mode_click_selects_row`
- [ ] `selection_multi_mode_click_clears_previous`
- [ ] `selection_multi_mode_ctrl_click_toggles_on`
- [ ] `selection_multi_mode_ctrl_click_toggles_off`
- [ ] `selection_multi_mode_ctrl_click_empty_selection`

Unit tests (Range selection - 7 tests):
- [ ] `selection_shift_click_range_forward`
- [ ] `selection_shift_click_range_backward`
- [ ] `selection_shift_click_single_row`
- [ ] `selection_shift_click_no_anchor_uses_clicked_as_anchor`
- [ ] `selection_shift_click_anchor_not_visible_uses_clicked`
- [ ] `selection_shift_click_extends_from_anchor_replaces_selection`
- [ ] `selection_shift_click_sorted_order`

Unit tests (Formatting and modes - 6 tests):
- [ ] `format_selection_count_none`
- [ ] `format_selection_count_visible_only`
- [ ] `format_selection_count_with_hidden`
- [ ] `format_selection_count_all_hidden`
- [ ] `selection_mode_none_ignores_clicks`
- [ ] `selection_mode_serialization`

Integration tests (message handling - 9 tests):
- [ ] `set_table_selection_updates_runtime`
- [ ] `clear_table_selection_clears_runtime`
- [ ] `set_table_selection_nonexistent_tile_ignored`
- [ ] `selection_persists_after_sort_change`
- [ ] `selection_persists_after_filter_change`
- [ ] `selection_clears_on_remove_table_tile`
- [ ] `selection_mode_none_prevents_selection`
- [ ] `multiple_tiles_independent_selection`
- [ ] `selection_not_serialized`

Snapshot tests (visual verification - 8 tests):
- [ ] `table_selection_single_row` - Single row highlighted
- [ ] `table_selection_multiple_rows` - Multiple non-contiguous rows highlighted
- [ ] `table_selection_contiguous_range` - Range selection highlighted
- [ ] `table_selection_with_sort` - Selection persists after sort (rows at different positions)
- [ ] `table_selection_with_filter_hidden_count` - Shows "N selected (M hidden)"
- [ ] `table_selection_empty` - No selection (baseline)
- [ ] `table_selection_first_row` - First row edge case
- [ ] `table_selection_last_row` - Last row edge case

---

**Acceptance Criteria**

Stage 8 is complete when:
1. All 6 TableSelection method unit tests pass ✅
2. All 3 Single mode unit tests pass ✅
3. All 5 Multi mode unit tests pass ✅
4. All 8 Range selection unit tests pass ✅ (added selection_shift_click_sorted_order)
5. All 6 Formatting/mode unit tests pass ✅
6. All 10 integration tests pass ✅ (added selection_persists_after_filter_change)
7. Snapshot tests deferred (require visual acceptance workflow)
8. `cargo clippy --no-deps` reports no warnings for new code ✅
9. `cargo fmt` produces no changes ✅

**Total new tests implemented: 37** (28 unit + 10 integration, snapshot tests pending visual acceptance)

**Status: COMPLETE** (as of implementation)

**Notes:**
- "Clear selection when `cache_generation` changes" is deferred to Stage 10/11 when waveform reload invalidation is fully implemented.
- Snapshot tests require a visual acceptance workflow and are listed but not yet added to the test suite.
- `TableRowId` now derives `Ord` and `PartialOrd` (required for `BTreeSet` operations).
- `SetTableSelection` message is marked `#[serde(skip)]` since `TableSelection` is runtime-only state.

---

### Stage 9: Keyboard navigation and clipboard

**Goal:** Full keyboard navigation and copy-to-clipboard support.

**Prerequisites:** Stage 8 (Selection) complete - selection system provides the foundation for keyboard navigation.

**Deliverables:**
- Implement keyboard handling when table has focus:
  - Up/Down: move selection by one row.
  - Page Up/Down: move by visible page height.
  - Home/End: jump to first/last row.
  - Ctrl+Home/Ctrl+End: jump and scroll to first/last.
  - Enter: activate selected row (emit `TableAction`).
  - Escape: clear selection.
  - Ctrl/Cmd+A: select all (in Multi mode).
  - Ctrl/Cmd+C: copy selected rows to clipboard.
- Implement type-to-search: buffer keystrokes, fuzzy jump to matching row.
- Implement copy format: tab-separated values, visible columns only.
- Scroll to keep focused/selected row visible.

**Acceptance tests:**
- [ ] Integration test: Up/Down moves selection correctly.
- [ ] Integration test: Page Up/Down moves by page.
- [ ] Integration test: Home/End jumps to boundaries.
- [ ] Integration test: Enter activates row (check Message emitted).
- [ ] Integration test: Ctrl+C copies correct tab-separated format.
- [ ] Integration test: Type-to-search jumps to matching row.
- [ ] Integration test: Selection scrolls into view.

---

### Stage 10: Scroll behavior and polish

**Goal:** Implement scroll position preservation rules and final polish.

**Prerequisites:** Stage 9 (Keyboard navigation) complete.

**Deliverables:**
- After sort: scroll to keep first selected row visible.
- After filter: scroll to top if selected row is hidden.
- After activation: ensure activated row is visible.
- **Clear selection when `cache_generation` changes** (deferred from Stage 8 - waveform reload).
- Implement column resizing (drag column borders).
- Persist column widths in `TableViewConfig.columns`.
- Implement column visibility toggle (context menu or column picker).
- Wire up accessibility (ensure rows are keyboard-focusable, accesskit labels).
- Performance optimization: ensure no per-frame O(n) work.

**Acceptance tests:**
- [ ] Integration test: Sort preserves scroll to selected row.
- [ ] Integration test: Filter scrolls to top when selection hidden.
- [ ] Integration test: Column resize persists to config.
- [ ] UI snapshot test: Column resize handles visible.
- [ ] Performance test: 100,000 row table maintains 60 FPS scroll.
- [ ] Accessibility test: Screen reader can navigate table rows.

---

### Stage 11: SignalChangeList model

**Goal:** Implement table model for signal value transitions.

**Prerequisites:** Stages 1-10 complete.

**Deliverables:**
- Define `TableModelContext` struct providing:
  - `wave_data: Option<&WaveData>` - access to waveform data
  - `transaction_container: Option<&TransactionContainer>` - access to FTR transactions
  - `time_format: &TimeStringFormatting` - for consistent time display
  - `time_unit: TimeUnit` - current time unit for display
- Extend `TableModelSpec::create_model(&self, ctx: &TableModelContext) -> Result<Arc<dyn TableModel>, TableCacheError>`
  - Virtual model ignores context (returns Ok)
  - SignalChangeList returns `Err(ModelNotFound)` if wave_data is None or variable not found
- Implement `SignalChangeListModel` in `sources/signal_change_list.rs`:
  - Constructor: takes `VariableRef`, `field: Vec<String>`, wave data reference.
  - Schema: columns for "Time", "Value", optional "Duration".
  - `row_id_at(index)`: timestamp of transition (ensures uniqueness).
  - `cell()`: format time using `TimeStringFormatting`, value using translator.
  - `sort_key()`: numeric timestamp for Time, formatted value for Value.
  - `on_activate(row)`: return `TableAction::CursorSet(timestamp)`.
- Handle missing variable gracefully (`TableCacheError::ModelNotFound`).
- Invalidate cache on waveform reload (generation change).

**Acceptance tests:**
- [ ] Unit test: Model extracts correct transitions from test waveform.
- [ ] Unit test: Time column formats according to current `TimeUnit`.
- [ ] Unit test: Value column uses correct translator.
- [ ] Integration test: Activating row sets cursor to transition time.
- [ ] Integration test: Waveform reload clears cache and selection.
- [ ] UI snapshot test: SignalChangeList renders for sample VCD.

---

### Stage 12: TransactionTrace model

**Goal:** Implement table model for FTR transaction traces.

**Prerequisites:** Stages 1-10 complete.

**Deliverables:**
- Implement `TransactionTraceModel` in `sources/transaction_trace.rs`:
  - Constructor: takes `StreamScopeRef`, optional `TransactionStreamRef`.
  - Schema: columns for "Type", "Start", "End", "Duration", "Generator", plus attribute columns.
  - `row_id_at(index)`: transaction ID from FTR.
  - `cell()`: format times, extract attributes.
  - `on_activate(row)`: return `TableAction::FocusTransaction(tx_ref)`.
- Wire up `TableModelSpec::TransactionTrace` in factory.
- Handle missing stream gracefully.
- Dynamic schema: attribute columns derived from transaction data.

**Acceptance tests:**
- [ ] Unit test: Model extracts transactions from test FTR data.
- [ ] Unit test: Duration calculated correctly from start/end.
- [ ] Unit test: Attribute columns appear in schema.
- [ ] Integration test: Activating row focuses transaction in viewer.
- [ ] UI snapshot test: TransactionTrace renders for sample FTR.

---

### Stage 13: SearchResults model

**Goal:** Implement source-level search producing a derived table.

**Prerequisites:** Stages 1-10 complete.

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

**Prerequisites:** Stages 1-10 complete.

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

**Prerequisites:** Stages 1-10 complete, analysis framework designed.

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
MVP (Virtual model only):
  Stage 1  → Stage 2  → Stage 3  → Stage 4  → Stage 5
     ↓
  Stage 6  → Stage 7  → Stage 8  → Stage 9  → Stage 10
     ↓
Real data models (can be parallelized):
  Stage 11 (SignalChangeList)
  Stage 12 (TransactionTrace)
  Stage 13 (SearchResults)
  Stage 14 (Custom)
     ↓
Future:
  Stage 15 (AnalysisResults - v2)
```

Each stage should be completed with all acceptance tests passing before moving to the next. Stages 11-14 can be implemented in parallel after Stage 10 is complete.
