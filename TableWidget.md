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
- Implement `SurferTileTree::add_table_tile(spec) -> TableTileId`.
- Update `SurferTileBehavior::pane_ui()` to dispatch to `draw_table_tile()`.
- Update `SurferTileBehavior::is_tab_closable()` to allow closing table tiles.
- Update `SurferTileBehavior::on_tab_close()` to clean up table tile state.
- Add `Message::AddTableTile` and `Message::RemoveTableTile` handlers.

**Acceptance tests:**
- [ ] Integration test: `AddTableTile` creates tile visible in tile tree.
- [ ] Integration test: Closing table tile removes it from `table_tiles` and `table_runtime`.
- [ ] Serialization test: Save state with table tile, reload, tile config preserved.
- [ ] Serialization test: Runtime state (selection, scroll) is NOT serialized.
- [ ] Unit test: `TableTileId` generation produces unique IDs.

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
- [ ] UI snapshot test: Virtual table (10 rows, 3 columns) renders correctly.
- [ ] UI snapshot test: Virtual table (1000 rows) renders without lag (virtualization working).
- [ ] UI snapshot test: Table with `dense_rows: true` has reduced row height.
- [ ] UI snapshot test: Loading state shows spinner.
- [ ] UI snapshot test: Error state shows error message.
- [ ] Manual test: Vertical scroll is smooth with 10,000 rows.

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

**Acceptance tests:**
- [ ] Unit test: Stable sort preserves original order for equal keys.
- [ ] Unit test: Multi-column sort applies correct priority.
- [ ] Integration test: Click column header updates sort and rebuilds cache.
- [ ] Integration test: Shift+click adds secondary sort without clearing primary.
- [ ] Integration test: Click without Shift resets to single-column sort.
- [ ] UI snapshot test: Sort indicators render correctly (▲1, ▼2).

---

### Stage 7: Display filter (search)

**Goal:** Implement view-level filtering with contains/regex/fuzzy modes.

**Deliverables:**
- Add filter input UI above table (text field + mode selector + case toggle).
- Emit `Message::SetTableDisplayFilter` on filter change.
- Implement message handler to update `TableViewConfig.display_filter`.
- Invalidate cache when filter changes.
- Implement filtering in cache builder:
  - Contains: substring match on `search_text`.
  - Regex: compile pattern, match against `search_text`.
  - Fuzzy: implement simple fuzzy matching (subsequence).
- Cache compiled regex to avoid repeated compilation.
- Show filter badge indicating active filter.
- Show row count: "Showing N of M rows".

**Acceptance tests:**
- [ ] Unit test: Contains filter matches substring correctly.
- [ ] Unit test: Regex filter matches pattern correctly.
- [ ] Unit test: Fuzzy filter matches subsequence correctly.
- [ ] Unit test: Case-insensitive matching works.
- [ ] Unit test: Invalid regex returns `TableCacheError::InvalidSearch`.
- [ ] Integration test: Filter change rebuilds cache with filtered rows.
- [ ] UI snapshot test: Filter input and badge render correctly.
- [ ] UI snapshot test: "Showing N of M rows" displays correct counts.

---

### Stage 8: Selection

**Goal:** Implement single and multi-row selection with persistence across sort/filter.

**Deliverables:**
- Implement `TableSelection` in runtime state.
- Single-click row: select (clear previous in Single mode, toggle in Multi mode).
- Shift+click: range selection from anchor.
- Ctrl/Cmd+click: toggle selection without clearing.
- Highlight selected rows with theme selection color.
- Track selection by `TableRowId`, not by index.
- Selection persists when sort changes (rows reorder but selection preserved).
- Selection persists when filter hides rows (hidden rows stay selected).
- Show selection count: "N selected" or "N selected (M hidden)".
- Clear selection when `cache_generation` changes (waveform reload).

**Acceptance tests:**
- [ ] Unit test: Single mode replaces selection on click.
- [ ] Unit test: Multi mode toggles on Ctrl+click.
- [ ] Unit test: Range selection selects all rows between anchor and target.
- [ ] Integration test: Selection persists after sort.
- [ ] Integration test: Selection persists for filtered-out rows, count shows "(M hidden)".
- [ ] Integration test: Selection clears on generation change.
- [ ] UI snapshot test: Selected rows are highlighted.

---

### Stage 9: Keyboard navigation and clipboard

**Goal:** Full keyboard navigation and copy-to-clipboard support.

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

**Deliverables:**
- After sort: scroll to keep first selected row visible.
- After filter: scroll to top if selected row is hidden.
- After activation: ensure activated row is visible.
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
- Implement `SignalChangeListModel` in `sources/signal_change_list.rs`:
  - Constructor: takes `VariableRef`, `field: Vec<String>`, wave data reference.
  - Schema: columns for "Time", "Value", optional "Duration".
  - `row_id_at(index)`: timestamp of transition (ensures uniqueness).
  - `cell()`: format time using `TimeStringFormatting`, value using translator.
  - `sort_key()`: numeric timestamp for Time, formatted value for Value.
  - `on_activate(row)`: return `TableAction::CursorSet(timestamp)`.
- Wire up `TableModelSpec::SignalChangeList` in factory.
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
