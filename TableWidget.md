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
- Async caching with invalidation: expensive operations (sorting, search, row index construction) run in a worker thread. Cache entries are keyed by (model identity + view search + sort + generation) and invalidated when wave data reloads.
- In-flight dedupe: at most one cache build per (tile_id, cache_key) runs at a time.
- Cache adoption guard: TableCacheBuilt is only applied if (tile_id exists, cache_key matches current view state, and generation matches). Stale results are dropped to avoid UI races after sort/search changes or tile close.
- Stable row identity: selection uses a stable TableRowId (not an index) so selections survive sorting/filtering and incremental updates.
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
- Selection state is runtime-only (not serialized); TableViewConfig only stores the selection mode.

## API

### New types

```rust
/// Unique identifier for a table tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableTileId(pub u64);

/// Stable row identity for selection and caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableRowId(pub u64);

/// Serializable model selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TableModelSpec {
    SignalChangeList { variable: VariableRef, field: Vec<String> },
    TransactionTrace { stream: StreamScopeRef, generator: Option<TransactionStreamRef> },
    SearchResults { query: TableSearchSpec },
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
    pub search: TableSearchSpec,
    pub selection: TableSelectionMode,
    pub dense_rows: bool,
    pub sticky_header: bool,
}

/// Runtime, non-serialized cache handle.
pub struct TableCacheEntry {
    inner: OnceLock<TableCache>,
    pub cache_key: TableCacheKey,
    pub generation: u64,
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
    SetTableSearch { tile_id: TableTileId, search: TableSearchSpec },
    SetTableSelection { tile_id: TableTileId, selection: TableSelection },
    BuildTableCache { tile_id: TableTileId, cache_key: TableCacheKey },
    TableCacheBuilt { tile_id: TableTileId, entry: Arc<TableCacheEntry>, result: Result<TableCache, String> },
}
```

### Type glossary (v1)

- TableSchema: ordered list of columns with stable keys and labels, with optional default width/visibility hints. Keys are strings or u64s.
- TableColumnConfig: view layout for a column key (key, width, visibility, resizable). No formatting or alignment overrides in v1.
- TableSortSpec: (column key, direction) list used for multi-column sorting.
- TableSearchSpec: { mode, case_sensitive, text }. Empty text means no filter.
- TableSelectionMode: None | Single | Multi.
- TableSelection: runtime selection state (set of TableRowId + anchor for range selection).
- TableCell: display-ready cell content (string or rich text), optional tooltip.
- TableSortKey: sortable value (numeric/string/bytes) used for stable ordering.
- TableCacheKey: { model_key, view_search, view_sort, generation }. model_key is derived from TableModelSpec (including source-level search).
- TableCache: cached row_ids in display order + per-row search text + per-row sort keys.
- TableAction: activation result (CursorSet, FocusTransaction, SelectSignal, None).

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

1. Table view attempts to access a cache entry by key (model identity + view search + sort + generation).
2. If the cache is missing or stale, the view renders a loading state and emits Message::BuildTableCache.
3. The SystemState handler starts a worker (perform_work) that builds the row index and any search/sort metadata. Cache building is always off-thread.
4. If a build for the same cache_key is already in-flight, new BuildTableCache requests are ignored to avoid duplicate work.
5. On completion, the worker sends Message::TableCacheBuilt and decrements OUTSTANDING_TRANSACTIONS.
6. The cache entry stores the built TableCache in a OnceLock only if tile_id still exists and the cache_key + generation still match the current view state; otherwise the result is dropped. The next frame renders the table normally when a valid cache exists.
7. If cache building fails, the error string is stored in runtime state for the tile and displayed in the UI until the next successful build or query change.

This mirrors analog cache behavior (AnalogVarState + AnalogCacheEntry + cache_generation).

### Search and sort

- Search uses a TableSearchSpec similar to VariableFilter: mode (contains, regex, fuzzy), case sensitivity, and an input string.
- Regex compilation is cached to avoid per-frame rebuilds; invalid regex yields an error state in the UI.
- Sorting is stable and multi-column (primary, secondary) and uses cached sort keys to avoid repeated string formatting.
- Search scope split: TableModelSpec::SearchResults is a source-level search (built from underlying waveform data by a worker), producing a derived table model. TableViewConfig.search is a view-level filter applied to the current table cache (post-source search). Both can be active: source search narrows the dataset; view search further filters rows in the cache. The UI should show both scopes distinctly to avoid confusion.
- Cache build stores per-row search text and sort keys; TableModel::search_text is called only during cache build, not per frame.

### Selection and activation

- Selection is stored as TableSelection (single/multi). It is keyed by TableRowId to survive sorting.
- Keyboard navigation uses Up/Down to move selection, Enter to activate, and Ctrl/Cmd-C to copy row text.
- Activation returns a TableAction, mapped to Messages like CursorSet or FocusTransaction.
- Selection lifecycle: on cache_generation change, selection is cleared because row ids may no longer be valid across reloads. If a model can provide stable ids across reloads in the future, a remap hook can preserve selection.

### Responsiveness and accessibility

- Use egui_extras::TableBuilder with vscroll(true) and row virtualization.
- Column widths persist in TableViewConfig; columns can be resizable and auto-sized.
- Rows are drawn with selectable labels for accessibility and keyboard focus.
- Respect theme colors (SurferConfig theme) and ui zoom factor.

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
- Column widths/visibility do not affect cache_key; only model identity + view search + sort + generation do.

### Missing requirements from codebase review

- Use existing time formatting (TimeStringFormatting, TimeUnit) and translator logic to match waveform values.
- Handle waveform reload by invalidating caches via cache_generation (same pattern as analog).
- Avoid storing caches in undo/redo stacks; use manual Clone to drop runtime caches.
- Integrate with Message-based state updates to keep UI changes consistent with existing event flow.
- Support DataContainer::Empty and partially loaded waveforms with clear loading/empty states.
- Wire table actions into command_parser and keyboard_shortcuts (search focus, close tile, copy row).

## Testing strategy

- Unit tests for TableModel implementations (row_count, sort_key, search_text correctness).
- Cache tests: build, reuse, invalidate on generation change; ensure no stale results.
- Search tests: contains/regex/fuzzy; invalid regex reports error but does not panic.
- Sorting tests: stable ordering, multi-column sort.
- Selection tests: selection persists across sorting/filtering and clears on generation change.
- Serialization tests: TableTileState round-trip in .ron; caches omitted and rebuilt.
- UI snapshot tests: add a virtual table tile and verify rendering and scroll behavior.
- Performance tests: large virtual data sets, ensure no per-frame O(n) work and acceptable FPS.
