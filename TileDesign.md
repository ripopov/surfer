# Tiles & Tabs in Surfer — Design

Status: revised proposal, to be implemented on the `vtr` branch.
Code snippets specify contracts; exact library adapter signatures are verified during implementation.
Baseline: `db1ca915` (egui 0.36.1, y-location cache rework included).
Reference: `origin/table-ftr-event-vibes` (PoC, egui 0.35, `egui_tiles` 0.16) — ideas only, not a baseline.

---

## 1. Summary

Surfer gets a VSCode-like workspace: the central area is a tree of **splits** and
**tab groups** whose leaves are **tiles**. A tile is one presentation of the
loaded (immutable) data: a waveform view, a memory viewer, a marker table, a log
panel, later signal tables and pipeline views. Tiles can be created, split,
dragged into other groups, tabbed, closed, and focused with mouse or keyboard.
Tile commands act on an explicit target captured from focus or their originating view. The whole layout round-trips
through the `.surf.ron` state file.

Core decisions:

* `UserState` becomes tile-native: `layout` (the tree), `tiles` (tile entries by
  id), `item_lists` (what waveform tiles display, shareable between tiles), and
  `waves` (the shared document: data container, cursor, markers, time range).
* The proposed layout engine is `egui_tiles` 0.17 at runtime; Surfer owns the
  serialized layout format and the tile identity, so the file format does not
  depend on the crate.
* A tile kind is a struct implementing one trait (`TileView`) plus one variant
  in a registry enum (`TileKind`). Its state, rendering, messages and
  serialization live in its own module. The layout core never matches on a
  concrete kind.
* Persistent workspace state changes through commands. Tile `ui` takes `&self`;
  rendering may update only disposable caches and interaction scratch state.
* Existing floating/side widgets that present data (memory viewer, markers,
  logs, frame buffer, annotation list, transaction details) become tile kinds.
  Chrome (menu, toolbar, statusbar, overview, hierarchy sidebar) and transient
  dialogs stay outside the tree.

Non-goals: multiple loaded files ("sources"), editing data, floating/detached
windows, per-tile toolbars beyond what a kind draws itself.

---

## 2. Where we start

What exists today, and what stands in the way (file references are to the
current tree):

| Today | Where | Problem for tiles |
|---|---|---|
| One `WaveData` holds data **and** presentation: `items_tree`, `displayed_items`, `viewports: Vec<Viewport>`, `cursor`, `markers`, `focused_item`, `scroll_offset`, `annotations`, `graphics`, `drawing_infos` | `wave_data.rs:56-110` | Presentation must be per tile; data and cursor must stay shared |
| Multiple viewports = extra `Panel::right("view port {idx}")` sharing one name/value column and one item list | `view.rs:479-508` | Addressed by bare `usize`; only push/pop at the tail; no identity |
| `viewport_idx: usize` in 7 messages, positional `usize` in 4 more, hard-coded `0` in ~12 places | `message.rs`, `keys.rs:118/134/197`, `keyboard_shortcuts.rs:461/470/522`, `time.rs:747`, `overview.rs:100`, `wcp_handler.rs:248/255` | Needs one targeting type |
| Draw cache `draw_data: RefCell<Vec<Option<CachedDrawData>>>` indexed by viewport, plus one shared `last_canvas_rect` | `system_state.rs:71,97`, `drawing_canvas.rs:711-716` | Regenerates every frame with ≥2 viewports; not resized on state load (panics on a file saved with >1 viewport) |
| Panels use global string ids (`"variable list"`, `"variable values"`) and unsalted `ScrollArea`s | `view.rs:353,395,411,426` | Two tiles would share egui panel state |
| Floating windows keyed by title, driven by singleton flags (`show_logs`, `show_cursor_window`, `memory_viewer.open`, `frame_buffer_content`) | `state.rs:98-110`, `system_state.rs:113-117` | One instance each; not part of the layout; not saved as layout |
| No layout in the state file; panel widths live in egui memory only | `state.rs`, `config.rs:208-291` | New serialized surface needed |
| Undo (`CanvasState`) snapshots items, markers, annotations only | `lib.rs:293`, `state.rs:597-612` | Decide whether tile open/close is undoable |
| Widgets never mutate state; they push `Message`s, drained in `App::ui` | `view.rs:105-132` | Keep this invariant; the PoC broke it for one tile kind |

Assets to reuse, adapting ownership and call sites where needed: `Viewport` (`Copy`, serde, relative time),
`DisplayedItemTree`/`DisplayedItem`/`DisplayedItemRef` and the reattachment logic
in `WaveData::update_with_items`, the y-location cache
(`compute_item_drawing_infos` / `item_layout_signature`), the analog cache
(viewport-independent), `MessageTarget<T>`, the config-driven shortcut table,
the snapshot test harness.

---

## 3. Overview

```
UserState
├── layout: Layout            ─ tree of Split / Tabs / Tile(TileId); focused tile
├── tiles: BTreeMap<TileId, TileEntry>
│     ├── 1 → kind: Waveform(WaveformTile { items: ItemListId(1), viewport, scroll, focus… })
│     ├── 2 → kind: Waveform(WaveformTile { items: ItemListId(1), viewport, … })   ← linked
│     ├── 3 → kind: Waveform(WaveformTile { items: ItemListId(2), … })             ← independent
│     ├── 4 → kind: Memory(MemoryTile { scope, name, formats, filters… })
│     └── 5 → kind: Markers(MarkersTile {})
├── item_lists: BTreeMap<ItemListId, ItemList>
│     ├── 1 → ItemList { items_tree, displayed_items, annotations, graphics, … }
│     └── 2 → ItemList { … }
└── waves: Option<WaveData>   ─ shared document: DataContainer, source, cursor, markers,
                                active_scope, time range, analog caches
```

Ownership rules:

* **Shared document** (`WaveData`): things that are true regardless of which
  tile you look through. Loaded data, cursor, marker times, the hierarchy
  selection, time unit, time range.
* **Item list** (`ItemList`): everything anchored to displayed items. A
  waveform tile displays exactly one list; several tiles may display the same
  list ("linked" tiles — today's multi-viewport feature, generalized).
* **Tile**: everything that belongs to one view: zoom/pan (`Viewport`),
  vertical scroll, focused item, focused transaction, column visibility, a
  user-given title, and for non-waveform kinds all their settings.
* **Runtime** (never serialized): draw caches, y-location cache, in-flight
  async work. Lives on the owning struct but is absent from file DTOs.

---

## 4. Core types

Module layout:

```
libsurfer/src/tiles/
├── mod.rs        re-exports; TileId, ItemListId, TileTarget
├── layout.rs     Layout, LayoutNode, split/tabs/move/close operations, focus navigation
├── kind.rs       TileKind + TileMessage registry and context-aware factories
├── view.rs       TileView trait, TileCtx
├── render.rs     egui_tiles::Behavior impl, draw_layout, tab bar chrome, focus detection
├── serde.rs      versioned file DTOs, validation, legacy migration
├── commands.rs   target resolution, workspace transactions, undo records
└── runtime.rs    session allocators, workspace epochs, async request tokens
libsurfer/src/tile_kinds/
├── waveform.rs   WaveformTile (moves canvas/name/value column code out of view.rs)
├── memory.rs     MemoryTile (from memory_viewer.rs)
├── markers.rs    MarkersTile (from marker.rs::draw_marker_window)
├── logs.rs       LogsTile (from logs.rs::draw_log_window)
├── frame_buffer.rs, annotation_list.rs, transaction_details.rs
└── …             future: signal_table.rs, pipeline.rs
```

### 4.1 Identity

```rust
/// Stable identity of a tile within one state. Never reused within a session.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId(pub u64);

/// Identity of an item list. Several waveform tiles may reference the same list.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemListId(pub u64);
```

Allocation lives in `SystemState` runtime state, outside serialization and undo.
Tile and item-list allocators are monotonic for the lifetime of the application:
loading a workspace advances each allocator to at least `max(loaded keys) + 1`
and never decreases it. Allocation uses checked arithmetic; exhausted or invalid
IDs produce a load/command error. Undo may restore the same logical tile under
its old ID; it never assigns that ID to a different tile.

Persisted IDs are local to a workspace. Loading another workspace may contain
the same numeric IDs, so a runtime `WorkspaceEpoch` is advanced whenever a
workspace is replaced, including loading a state file. Document replacement or
reload separately advances `DocumentGeneration`. Neither counter is restored
by undo. Egui IDs include the workspace epoch and application `TileId`.

Every async request carries `(workspace_epoch, document_generation, tile_id,
request_id)` plus its complete input key. Each new request gets a session-wide
monotonic request ID. Accept a completion only if the workspace/document still
match, the tile exists, and its pending request and input key match. Closing,
undo-restoring or reconfiguring a tile clears its pending request. Cancellation
is an optimization; rejecting stale completions is mandatory.

`egui_tiles::TileId` (the crate's node id) is a runtime detail and is never
serialized or exposed in messages. Panes in the runtime tree carry our `TileId`.

### 4.2 Layout

```rust
pub struct Layout {
    /// Authoritative layout, changed only by workspace commands.
    root: Option<LayoutNode>,
    /// Runtime adapter and interaction state; rebuilt/reconciled from root.
    tree: egui_tiles::Tree<TileId>,
    /// The tile that receives ambient commands and keyboard input.
    pub focused: Option<TileId>,
    /// Most-recently-focused first. Used to pick "the active waveform tile"
    /// when the focused tile is of another kind.
    pub focus_history: Vec<TileId>,
}

/// Serialized shape of the tree. This is the file format; see §8.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Tile(TileId),
    Split { dir: SplitDir, shares: Vec<f32>, children: Vec<LayoutNode> },
    Tabs { active: usize, children: Vec<LayoutNode> },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDir { Horizontal, Vertical }
```

Persistence uses an explicit `LayoutFile { root, focused, focus_history }` DTO;
`Layout` itself is not the file format. `LayoutNode` is the authoritative tree.
The runtime adapter maps its nodes to `egui_tiles` containers and panes, keeping
node IDs stable across frames when the corresponding node survives.

Only linear splits and tab groups are supported. Disable grid creation in the
adapter; an unexpected runtime grid is an adapter error, not a lossy conversion
to a horizontal split. Tabs contain tile leaves; splits contain splits or tab
groups. Normalize bare leaves into single-tab groups. Preserve a single-tab
group even when simplifying; remove empty containers and redundant splits.

Validate the whole workspace atomically on load and on `SetLayout`: each tile
appears exactly once, each referenced tile/list exists, and every list has an
owner (except conservatively retained unknown-kind resources, §8.2). Validate that
tab indices are in range, split shares match child counts and are finite and
positive, and focus/history refer to existing tiles without duplicates. Bound parser
recursion and input size, then enforce node-count/depth limits before conversion. Report invalid
input without replacing the current workspace. Normalize shares and discard
stale saved focus/history entries as explicitly documented repairs; do not
silently invent missing tiles or item lists. Focus must be visible; explicitly
focusing a hidden tile activates its tab first.

The adapter returns a proposed layout edit after a UI pass. It cannot commit
persistent changes itself; §6.1 defines how proposals enter the command path.

Internal layout operations, called only by the validated workspace dispatcher:

```rust
impl Layout {
    pub fn tile_order(&self) -> Vec<TileId>;               // depth-first, left-to-right
    pub fn visible_tiles(&self) -> Vec<TileId>;            // active tab of every group
    pub fn insert(&mut self, tile: TileId, at: Placement);
    pub fn remove(&mut self, tile: TileId);                // then simplify()
    pub fn move_tile(&mut self, tile: TileId, to: Placement);
    pub fn set_active_tab(&mut self, tile: TileId);        // makes tile visible in its group
    pub fn neighbor(&self, from: TileId, dir: Direction) -> Option<TileId>;  // spatial, by rect
    pub fn next_in_group(&self, from: TileId, delta: isize) -> Option<TileId>;
    pub fn rect(&self, tile: TileId) -> Option<Rect>;      // revision-checked adapter geometry
    pub fn simplify(&mut self);                            // preserve single-tab groups
}

/// Where a new or moved tile goes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Placement {
    /// Add as a tab in the same group as `anchor`, right after it.
    TabAfter(TileId),
    /// Split `anchor`'s slot; the new tile takes `Direction` side.
    Beside(TileId, Direction),
    /// Split the whole layout; e.g. logs at the bottom.
    Edge(Direction),
    /// Empty layout only: become the root; reject if nonempty.
    Root,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Left, Right, Up, Down }
```

The tree is allowed to be empty (all tiles closed). The central area then
shows the welcome/splash content with a hint; loading a file into an empty
layout creates one waveform tile.

### 4.3 Tile kinds: the registry and the trait

A generic entry owns metadata common to all tile kinds:

```rust
pub struct TileEntry {
    pub title: Option<String>,
    pub kind: TileKind,
}
```

`tiles/kind.rs` is the **only** file that enumerates kinds:

```rust
#[derive(Clone)]
pub enum TileKind {
    Waveform(WaveformTile),
    Memory(MemoryTile),
    Markers(MarkersTile),
    Logs(LogsTile),
    FrameBuffer(FrameBufferTile),
    AnnotationList(AnnotationListTile),
    TransactionDetails(TransactionDetailsTile),
    /// A kind this build does not know (state file from a newer Surfer).
    Unknown(UnknownTile),
}

/// Kind-specific messages. `Message::ToTile(target, TileMessage)` routes them.
#[derive(Debug, Deserialize)]
pub enum TileMessage {
    Waveform(WaveformMessage),
    Memory(MemoryMessage),
    Markers(MarkersMessage),
    Logs(LogsMessage),
    FrameBuffer(FrameBufferMessage),
    AnnotationList(AnnotationListMessage),
    TransactionDetails(TransactionDetailsMessage),
}

impl TileKind {
    pub fn view(&self) -> &dyn TileView { match self { Self::Waveform(t) => t, Self::Memory(t) => t, /* … */ } }
    pub fn view_mut(&mut self) -> &mut dyn TileView { /* same */ }
    pub fn kind_name(&self) -> &'static str { self.view().kind_name() }
    /// Deliver a kind-specific message. Mismatched kind is a logged no-op.
    pub fn update(&mut self, msg: TileMessage, cx: &mut TileUpdateCtx) {
        match (self, msg) {
            (Self::Waveform(t), TileMessage::Waveform(m)) => t.update(m, cx),
            (Self::Memory(t), TileMessage::Memory(m)) => t.update(m, cx),
            (Self::Markers(t), TileMessage::Markers(m)) => t.update(m, cx),
            /* … */
            (t, m) => warn!("{} tile ignored {m:?}", t.kind_name()),
        }
    }
    // Registry metadata: stable name, singleton policy, payload version,
    // decoder/encoder and create(&mut TileCreateCtx) -> Result<TileKind>.
    // Waveform creation allocates a valid empty ItemList in the same transaction.
}
```

Use one explicit registry with enum dispatch; a macro is unnecessary initially.
The enum provides exhaustive internal dispatch, while dedicated file DTOs and
per-kind codecs implement persistence (§8). No runtime plugin registry is needed.
Factories receive the context they need: `WaveformTile::default()` cannot create
a valid list reference on its own. Singleton policy is registry metadata and is
enforced by every creation path, including split and file validation.

Waveform-local operations use `TileMessage::Waveform`, just like other kinds.
Move existing item, zoom and annotation operations into `WaveformMessage` and
update their callers together. Cursor and marker-time changes remain shared
`DocumentCommand`s; marker-row changes belong to an explicit item list.
Global application operations (load, preferences, dialogs) remain top-level.
Legacy textual command names may translate at the input boundary; the internal
message architecture has no compatibility exception for waveforms.

`TileMessage` contains deserializable user commands only. Async completions use
a separate internal `TileCompletion` registry with request tokens (§4.1), so
runtime `Arc` payloads do not become part of the injection/file API.

The trait each kind implements (`tiles/view.rs`):

```rust
pub trait TileView {
    /// Stable kind name: menus, palette, tab default title, docs.
    fn kind_name(&self) -> &'static str;

    /// Default tab title; the renderer applies TileEntry::title if set.
    fn title(&self, cx: &TileCtx) -> String;

    /// Draw the tile body. Immutable: all changes are sent as messages via `cx`.
    /// Per-frame scratch state lives in egui memory or runtime RefCell fields.
    fn ui(&self, ui: &mut egui::Ui, cx: &mut TileCtx);

    /// A copy suitable for "split": `None` means the kind cannot be split-cloned
    /// and the split menu entry is disabled for it.
    fn split_clone(&self) -> Option<TileKind> { None }

    /// Called after the shared document changed (reload, switch_file, new file).
    /// Reattach stable references and clear caches. Missing targets render an
    /// unavailable state while retaining settings; document changes do not close tiles.
    fn on_waves_changed(&mut self, change: WavesChange, cx: &mut TileUpdateCtx) {}

    /// Extra entries for the tab context menu (after the generic ones).
    fn tab_context_menu(&self, ui: &mut egui::Ui, cx: &mut TileCtx) {}

    /// Optional palette commands for this kind, resolved to this tile before queuing.
    fn commands(&self) -> Vec<CommandSpec> { Vec::new() }

    /// For tests: false while required async work is pending; failures are terminal.
    fn is_ready(&self, cx: &TileCtx) -> bool { true }
}

pub enum WavesChange { Loaded, Reloaded { keep_unavailable: bool }, Cleared }

pub struct TileCtx<'a> {
    services: TileReadServices<'a>, // document, list lookup, config and shared services
    pub tile_id: TileId,
    pub focused: bool,
    commands: &'a mut CommandSink,
}
impl TileCtx<'_> {
    pub fn waves(&self) -> Option<&WaveData>;
    pub fn config(&self) -> &SurferConfig;
    pub fn theme(&self) -> &SurferTheme;
    pub fn translators(&self) -> &TranslatorList;
    pub fn item_list(&self, id: ItemListId) -> Option<&ItemList>;
    pub fn send_document(&mut self, m: DocumentCommand);
    pub fn request(&mut self, request: TileWorkRequest);
    /// Shorthand for `Message::ToTile(self.tile_id, m)`.
    pub fn send_self(&mut self, m: TileMessage);
    /// Salted egui id for widgets inside this tile.
    pub fn id(&self, salt: impl Hash) -> egui::Id;
}

pub struct TileUpdateCtx<'a> {
    pub tile_id: TileId,
    pub waves: Option<&'a WaveData>,
    pub config: &'a SurferConfig,
    pub translators: &'a TranslatorList,
    // Private transaction service: checked access to the tile's owned resources,
    // before/after recording, invalidation and concretely targeted followups.
    transaction: &'a mut TileTransaction,
}
```

`TileTransaction` exposes checked list-edit operations for the current tile and
records content before mutation. Shared document changes go through document
commands; kinds do not get mutable access to all lists or the whole document.
Tile-local setting edits use a transaction helper that records the kind's
before/after undo payload when the operation is semantic, and omits navigation.
Read-only context is composed from disjoint services; there is no unrestricted
`&SystemState` escape hatch in the kind contract.

`ui` takes `&self` on purpose: it is what the rest of Surfer already does
(widgets push messages, `update` mutates). Disjoint services keep the tile pass
read-only (§6.1), and the command boundary makes undo and interaction tests
reproducible. The PoC's Konata tile mutated serialized state
during draw and paid for it.

### 4.4 Shared document: `WaveData` after the split

`WaveData` keeps:

```rust
pub struct WaveData {
    pub inner: DataContainer,
    pub source: WaveSource,
    pub format: WaveFormat,
    pub active_scope: Option<ScopeType>,     // hierarchy sidebar selection
    pub cursor: Option<BigInt>,              // shared across all tiles
    pub markers: HashMap<u8, BigInt>,        // marker times; rows live in item lists
    pub display_variable_indices: bool,
    // runtime
    pub old_max_timestamp: Option<BigInt>,
    pub cache_generation: u64,
    pub inflight_caches: HashMap<AnalogCacheKey, Arc<AnalogCacheEntry>>,
    pub cached_time_range: TimeRange,
}
```

Removed from `WaveData` and moved: `items_tree`, `displayed_items`,
`display_item_ref_counter`, `default_variable_name_type`, `annotations`,
`annotation_groups`, `annotation_counter`, `graphics`, `drawing_infos`,
`drawing_infos_signature`, `total_height` → `ItemList`;
`viewports`, `last_active_viewport_idx`, `scroll_offset`, `focused_item`,
`focused_transaction` → `WaveformTile`; annotation selection/menu scratch →
the relevant view; annotation-list visibility → presence of its tile.

`WaveData::update_with_waves` (reload/switch) keeps only shared fields; the
per-list and per-tile reattachment is driven from `SystemState::on_waves_loaded`
(§11.3).

### 4.5 Item list

```rust
#[derive(Default)]
pub struct ItemList {
    pub items_tree: DisplayedItemTree,
    pub displayed_items: HashMap<DisplayedItemRef, DisplayedItem>,
    pub ref_counter: usize,
    pub default_variable_name_type: VariableNameType,
    // anchored to items, so they belong here
    pub annotations: Vec<Annotation>,
    pub annotation_groups: Vec<AnnotationGroup>,
    pub annotation_counter: i32,
    pub graphics: HashMap<GraphicId, Graphic>,
    // runtime: y-location cache (was WaveData::drawing_infos); content-space only
    pub layout_cache: RefCell<ItemLayoutCache>,   // { signature: u64, infos: Vec<ItemDrawingInfo>, total_height: f32 }
}
```

Everything that today takes `&WaveData` to read items takes `&ItemList`
instead (`compute_variable_display_names`, `visible_drawing_infos`, …).
Mutations (`add_variables`, `remove_items`, `move_item`, reattachment) go through
the owning transaction. `DisplayedItemRef` is list-local; cross-list references
carry `(ItemListId, DisplayedItemRef)`. `VisibleItemIndex` is a derived position,
not stable identity; save focused items by `DisplayedItemRef` and sanitize them
after edits or reattachment.

The shared layout cache contains content-space row positions, never a tile's
scroll offset, clip rectangle or pixel origin. Its key includes all shared
row-height, translation and fold inputs. Any future per-tile row sizing moves
this cache to the tile or adds those inputs to a multi-entry cache.
`selected_annotation` and annotation-menu interaction state belong to the
view displaying them, not the shared list. Use domain IDs for saved references,
never persisted `egui::Id`s.

An item list with no referencing tile is dropped when the last tile closes
(with its content retained by the close undo record, §9), except for the
unknown-kind preservation rule in §8.2. Content-copy and split-clone methods
explicitly reset caches; do not derive cloning that copies runtime state.

### 4.6 Waveform tile

```rust
pub struct WaveformTile {
    pub items: ItemListId,
    pub viewport: Viewport,                       // zoom/pan; was WaveData::viewports[i]
    pub scroll_offset: f32,
    pub link_vertical_scroll: bool, // participants with the same ItemListId
    pub focused_item: Option<DisplayedItemRef>,
    pub focused_transaction: Option<StableTransactionRef>, // resolve against current document
    pub show_name_column: bool,                   // default true
    pub show_value_column: bool,                  // default true
    pub selected_annotation: Option<AnnotationId>, // view selection, stable domain ID
    // runtime
    draw_cache: RefCell<Option<WaveDrawCache>>,   // { canvas_rect, viewport, list_signature, data: CachedDrawData }
}
```

One waveform tile = one canvas plus its own name and value columns. Today's
"add viewport" (extra time axis sharing the name column) is expressed as a
**linked split**: a second waveform tile referencing the same `ItemListId`,
with `show_name_column = false` on the right-hand tile if the user wants the
old look. Two tiles on the same list show the same items, folds, selection and
markers; each has its own zoom, scroll and focused item. Independent scrolling
means hiding columns alone does not maintain row alignment. When
`link_vertical_scroll` is enabled, participants with the same list share the
content-space offset: scrolling one updates all in one non-undoable command.
Clamp to the largest offset valid for all visible participants, using the same
row heights and canvas header alignment. Joining the group adopts its offset;
independent splits leave it. Migration enables it for the old extra viewports.
Editing the list from either tile is visible in both.

The draw cache lives in the tile. Its key covers document generation, canvas
geometry, viewport, list/content revision, scroll, translator/config/theme
revision, and any cursor/selection input used by cached drawing. Keep overlays
that vary independently outside cached waveform commands where practical. `invalidate_draw_commands()` becomes:

* `invalidate_tile(id)` — after zoom/pan/resize of one tile;
* `invalidate_list(list_id)` — after an item edit: every tile whose `items == list_id`;
* `invalidate_all()` — reload, config, theme.

This removes shared-rectangle cache contention; verify actual cache hit rates
with multiple tiles before claiming a performance improvement.

### 4.7 Migrated widget kinds

| Kind | State (serialized) | Reads (shared) | Notes |
|---|---|---|---|
| `MemoryTile` | stable array path, formats, filter/search/highlight settings, `value_column_count`, `color_values` (all of `MemoryViewerState` minus `open`/scroll/selection); runtime: `RefCell<Option<MemoryViewerCache>>` | `waves.cursor`, `waves.inner` | Multiple instances allowed (different arrays). "Show Memory Viewer" in the item context menu opens one beside the focused tile. `on_waves_changed` reattaches by stable path; missing arrays show an unavailable state. |
| `MarkersTile` | none | `waves.cursor`, `waves.markers`, marker rows of the target waveform's list | Singleton. Replaces `show_cursor_window`. |
| `LogsTile` | `filter: LevelFilter` | global log buffer | Singleton. "Open on error" becomes `WorkspaceCommand::OpenTile { kind: logs, placement: Edge(Down), focus: false }`. |
| `FrameBufferTile` | `FrameBufferSettings` (from `UserState::frame_buffer`) + which variable/array | `waves.cursor` | Replaces the `frame_buffer_content` window. |
| `AnnotationListTile` | `show_comments: bool` | annotations of the target waveform's list | Singleton. Replaces `show_annotation_list` right panel. |
| `TransactionDetailsTile` | none | `focused_transaction` of the target waveform tile | Singleton. Today it appears automatically when a transaction is focused; new behaviour: focusing a transaction opens it (once) at `Edge(Right)` if not open. |

Inspector tiles (markers, annotations, transaction details) may follow the
remembered waveform for display. At render time they capture that waveform/list
ID alongside row data, and any edit they emit carries those concrete IDs; they
never re-resolve the inspected list later. This is an explicit inspector binding,
not a general ambient-command fallback.

Stays outside the tree: menu, toolbar, statusbar, overview strip (draws one
rect per **waveform tile**, highlighting the target waveform tile), hierarchy
sidebar, command prompt, about/license/help/quickstart/gestures, performance
plot, load-URL and surver file windows, reload/sibling-state dialogs. These are
chrome or transient dialogs, not presentations of data.

### 4.8 Per-tile vs shared — reference table

| State | Scope | Rationale |
|---|---|---|
| Loaded data, source, format | shared | immutable document |
| Cursor | shared | "where am I" is a property of the session; all tiles (waveform, memory, markers) show the same instant |
| Marker times | shared | same |
| Marker rows, dividers, timelines, groups | item list | rows are items |
| Annotations, WCP graphics | item list | anchored to `DisplayedItemRef` |
| Selection, fold state | item list | stored in `DisplayedItemTree::Node`; linked tiles share it |
| Zoom/pan (`Viewport`) | tile | the whole point of a second view |
| Vertical scroll | tile | independent scrolling of linked tiles |
| Focused item | tile | keyboard focus is per view; validated against the list on use |
| Focused transaction | tile | |
| Active scope (sidebar) | shared | one sidebar |
| Time unit, time format, theme, show_* config overrides | shared (`UserState`) | global preferences |
| Draw commands, y-locations | runtime, per tile / per list | caches |

Cross-tile zoom **sync** (two waveform tiles following each other's time
window) is deliberately not in the first version; see §14.

---

## 5. Messages, commands, input

### 5.1 Targeting

Separate user intent from executable commands:

```rust
pub enum TileTarget { Id(TileId), Focused }
// Resolved command queue: no ambient targets remain.
pub enum Message {
    ToTile(TileId, TileMessage),
    ToDocument(DocumentCommand),
    Workspace(WorkspaceCommand),
    // application commands …
}
```

The input dispatcher resolves `Focused` exactly once, before enqueueing a
command, using the focused tile for generic/kind-specific actions. Waveform
commands from shared chrome may instead request the most recently focused
waveform. Make this fallback explicit in the command definition; it is not a
universal interpretation of `Focused`. A missing or wrong-kind explicit ID is
an error/no-op and never falls back to another tile.

Tile widgets always emit their own `TileId`. A handler resolves the tile's
`ItemListId` before list mutation. Every item reference is interpreted with
that list; cross-list operations carry `(ItemListId, DisplayedItemRef)`.
Followups retain their original concrete target. A stale target after a close
is ignored, not redirected. Shared cursor and marker-time commands need no tile.

`target_waveform()` chooses the focused waveform, otherwise the most recently
focused existing waveform, otherwise the first waveform in layout order. This
fallback is used only by declared waveform commands and following inspectors.
An ambient edit of a hidden waveform reveals it; read-only inspectors may follow
it without changing focus. Command metadata records whether this reveal occurs.

Only commands explicitly defined to create a view, such as adding variables
from the hierarchy, create a waveform when none exists. Zoom, remove-item and
other editing commands otherwise disable or report no target. Creation and the
initial operation form one transaction.

Focus events from a click/tab selection are ordered before ambient input from
the same interaction. Palette invocation captures its target until execution
or cancellation, so suggestion IDs and execution refer to the same item list.
Script batches resolve each statement against the state left by the previous
statement; they do not pre-resolve the entire batch against one initial focus.

### 5.2 Commands and transactions

Waveform-local commands (scroll, zoom, navigation, item edits, annotations,
column visibility) live in `WaveformMessage`, without a second tile target.
Remove `viewport_idx`, `AddViewport`, `RemoveViewport` and `SetActiveViewport`
from the internal API. Legacy command aliases are decoded at the boundary.

Workspace commands have concrete IDs and validated arguments:

```rust
CreateTile { kind: KindName, placement: Placement, focus: bool },
OpenTile { kind: KindName, placement: Placement, focus: bool },
CloseTile(TileId),
CloseOtherTiles(TileId),
FocusTile(TileId),
SplitTile { tile: TileId, dir: Direction, mode: SplitMode },
MoveTile { tile: TileId, to: Placement },
RenameTile { tile: TileId, title: Option<String> },
SetLayout(LayoutNode),
// Adapter-produced candidate, validated and committed by the same dispatcher.
ApplyLayoutEdit(LayoutEdit),
```

Creation accepts kind-specific initial settings through a typed creation spec
when needed (e.g. memory scope/path). It does not accept arbitrary runtime tile
objects. Factories allocate required resources in the transaction.
`OpenTile` reuses singleton kinds and honors `focus: false`; ordinary creation
also enforces singleton policy. Split modes are linked or independent for
waveforms, clone for kinds supporting it, otherwise disabled.

Each successful command validates invariants, applies all state changes,
invalidates affected caches and records one undo entry if appropriate. Failure
leaves the workspace unchanged. Closing removes the tile and its last-owned
list, updates focus/history and normalizes the tree. Choose the next visible
tab or spatial neighbor before removal; explicitly focusing a tile reveals it.
Closing the last tile leaves an empty workspace.

Linked waveform splits copy view settings and reference the same item list.
Independent splits clone list content under a new `ItemListId`; item IDs remain
local to the copied list. All split clones start with empty runtime caches and
no pending jobs. Both modes allocate a fresh tile ID.

Public injected commands are deserializable input DTOs which pass through the
same resolver and validator. Internal layout proposals and async completions
are not exposed as arbitrary serialized messages.

### 5.3 Commands (palette and `.sucl`)

Layout commands work without loaded data; data-dependent commands are disabled
until their inputs exist.

| Command | Resolved action |
|---|---|
| `tile_new <kind>` | Factory creation beside focused tile, or root when empty |
| `tile_split_right`, `tile_split_down` | Linked waveform split, or supported kind clone |
| `tile_split_copy_right`, `tile_split_copy_down` | Independent waveform split |
| `tile_close`, `tile_close_others` | Close captured tile / its group siblings |
| `tile_focus <id\|title>` | Reveal and focus identified tile |
| `tile_focus_left/right/up/down` | Focus visible spatial neighbor |
| `tile_next`, `tile_prev` | Activate adjacent tab in captured group |
| `tile_move_left/right/up/down` | Resolve neighbor/placement, then `MoveTile` |
| `tile_rename <name>` | Rename generic `TileEntry` |
| `tile_columns names\|values\|both\|none` | Waveform-local column command |
| `show_logs`, `show_marker_window`, `show_memory_viewer <array>`, `show_annotation_list` | Open/create through the registry |
| `viewport_add` | Split the resolved target waveform, linked |
| `viewport_remove` | Close the resolved target waveform |
| `viewport_set_active <n>` | Focus n-th waveform in layout order |

Existing `.sucl` spellings remain aliases only where their meaning is clear;
document changed behavior explicitly. Parser suggestions are generated from
the captured target and carry IDs scoped to its item list. Kind commands are
registered through `CommandSpec` and use the same target resolver.

### 5.4 Keyboard

Added to `ShortcutAction` / `SurferShortcuts` / `default_config.toml`
(`[shortcuts]`), so users can rebind them:

| Action | Default | Note |
|---|---|---|
| `tile_split_right` | `Command+Backslash` | as VSCode |
| `tile_split_down` | `Command+Shift+Backslash` | |
| `tile_close` | `Command+W` | browsers may swallow it in the wasm build; the palette and tab close button remain |
| `tile_next` / `tile_prev` | `Command+PageDown` / `Command+PageUp` | free today |
| `tile_focus_left/right/up/down` | `Command+Shift+ArrowLeft/Right/Up/Down` | free today |
| `tile_move_left/right/up/down` | `Command+Alt+ArrowLeft/…` | |
| `show_logs` | `Command+Shift+L` | free today |

Not used: `Ctrl+Tab` (browser), `Ctrl/Cmd+digit` (markers), `Alt+digit` (counts).

Routing (`keys.rs`): the existing guard (command prompt closed, no text edit
focused, `!egui_wants_keyboard_input`) stays. Tile-local keys: a kind that wants
keyboard handling reads `ui.input()` inside its `ui` **only when
`cx.focused`**, and only for keys not consumed by the global table (global
shortcuts run first, as today). The hard-coded fallbacks in `keys.rs`
(`J/K/H/L`, digits, arrows) use the input resolver and enqueue concrete targets.

The shortcut table declares target policy; individual key bindings need no
knowledge of tile kinds. Global shortcuts consume handled keys before tile UI.

### 5.5 Mouse

* Click, right-click or drag start inside a tile → `FocusTile(id)` (detected in
  `render.rs`, not per kind). Hover does **not** move focus.
* Wheel/pinch over a canvas → `ToTile(id, WaveformMessage::Scroll/Zoom)`
  for the hovered tile (as `viewport_idx` today). A tile can be zoomed without
  focusing it.
* Mouse gestures: `gesture_start_location`/`gesture_start_time` on
  `SystemState` gain `gesture_tile: Option<TileId>`; a release over another tile
  cancels the gesture.
* Tab drag-and-drop, split resizing, tab reordering: `egui_tiles`.
* Drag from the hierarchy: the drop target is the tile whose `layout.rect(id)`
  contains the pointer, if it is a waveform tile; else `target_waveform()`.
  `AddDraggedVariables` is sent to that explicit tile ID. The current
  `pointer.x > sidepanel_width` heuristic goes away.

### 5.6 WCP and wasm

WCP captures the **target waveform tile** at the start of each request: `get_item_list`,
`add_variables`, `focus_item`, `set_viewport_to`, `set_viewport_range` operate on
`target_waveform()`. `zoom_to_fit { viewport_idx }` keeps the field, interpreted
as the index into `tile_order()` filtered to waveform tiles; omitted → target.
Later protocol work may add an optional `tile` field; nothing in this design
blocks it. `DisplayedItemRef` in the protocol remains a per-list ref; clients
that manage several lists are out of scope.

`wasm_api::get_state` returns the versioned state DTO; `inject_message` validates
and resolves input commands at the boundary. `draw_text_arrow` targets
the target waveform's list.

---

## 6. Rendering and egui integration

### 6.1 Frame

`SystemState::draw` keeps the existing chrome order and renders the workspace
inside one central panel. Tile UI borrows document, lists and tile entries
immutably, and emits commands with explicit targets. Its `TileCtx` exposes
read-only services plus command/request submission; it must not expose a taken
or incomplete authoritative layout through `SystemState`.

The adapter owns a working `egui_tiles` tree reconciled from the authoritative
`LayoutNode`. During `tree.ui`, the library may mutate this working tree for
selection, docking, tab reordering or resizing. Record the resulting proposal;
do not write it directly into persistent workspace state. After the UI pass,
validate and commit the proposal through the workspace dispatcher, then apply
pane commands in recorded input order. A layout revision accompanies proposals;
reject/reconcile stale proposals instead of overwriting newer state.

Docking and tab reordering are structural edits and use the same undo policy as
keyboard moves. Coalesce a drag gesture into one transaction from drag start
to release, including frames where the library temporarily reparents nodes.
Cancelled drags discard the structural proposal. During a gesture, preview
geometry belongs to the adapter. Split resizing commits geometry changes without
an undo entry; tab activation/focus likewise uses non-undoable commands. Mixed
proposals must distinguish these changes, not classify the whole frame by a
single edit callback.

Adapter-owned pane rectangles are tagged with the current frame/layout revision.
Use current rendered rectangles for hit tests; hidden tiles have no active hit
rectangle. Focus navigation may use the last completed valid layout geometry.

Drawing helpers (`draw_items`, `draw_item_list`, `draw_var_values`,
`generate_draw_commands`) become immutable except for disposable caches in
`RefCell`s. Document reads and tile rendering never encounter temporarily empty
content maps or an empty authoritative layout.

### 6.2 `Behavior` implementation (`tiles/render.rs`)

The adapter implements these behaviors (verify exact `egui_tiles` signatures
against the pinned dependency when building the adapter):

* Render a pane by looking up `TileEntry`, constructing a read-only `TileCtx`
  and calling `entry.kind.view().ui(...)` under a clipped, salted UI.
* Detect click/right-click/drag-start and record focus before related commands.
* Compute titles once per frame; use entry title overrides generically.
* Intercept close buttons, enqueue `CloseTile(id)` and prevent immediate library
  removal. Context menus emit the same commands as the palette.
* Enable linear docking and tabs only. Track complete interaction boundaries
  and before/after topology rather than assuming an edit callback provides the
  selected pane or a complete transaction.
* Preserve one tab group per pane; prune empty groups and redundant splits.
* Report selection, geometry and structural proposals separately (§6.1).

`all_panes_must_have_tabs: true` means every tile always sits in a tab group,
so there is **one** rendering path whether there is one tile or ten. The only
thing that changes for a lone tile is `tab_bar_height` (config
`layout.hide_single_tab_bar`, default `true`). The PoC's separate
`is_single_waveform` fast path is not reproduced.

Tab context menu (generic part): Split Right, Split Down, Split Copy Right/Down
(waveform), Close, Close Others, Rename…, then `tab_context_menu` of the kind.

### 6.3 egui ids and clipping

* Every egui id inside a tile derives from `(workspace_epoch, TileId, salt)`,
  using an explicitly scoped pane `Ui`; runtime container IDs are not identity. Panels inside the waveform tile become
  `Panel::left(ui.id().with("names"))` and `Panel::left(ui.id().with("values"))`;
  every `ScrollArea` gets `id_salt(cx.id("…"))`. No global string ids inside
  tiles.
* Widget-focus bookkeeping keyed by string (`text_edit_focused`, `time_widgets`)
  uses the same workspace/tile namespace via a shared ID helper.
* Panels created inside a pane `Ui` do not inherit the pane's clip rect
  (documented in the PoC's `WaveClipIssue.md` for `egui_tiles` 0.16). The
  wrapper sets the clip rect once in `pane_ui`; the waveform tile re-applies
  `ui.set_clip_rect(ui.clip_rect().intersect(tile_rect))` inside its nested
  panel closures. Verify against `egui_tiles` 0.17 during implementation; keep
  the guard either way, it is cheap.
* The optional in-tile panels (annotation menu, focus-id list) must not shift
  auto ids of later widgets. Give each an explicit id; do not rely on
  `skip_ahead_auto_ids`.

### 6.4 Waveform tile body

`tile_kinds/waveform.rs::WaveformTile::ui` draws, inside the pane `Ui`:

1. `Panel::left(names)` — optional, resizable, `draw_item_list` with the tile's
   `scroll_offset`; writes `total_height` into the list's layout cache.
2. `Panel::left(values)` — optional; `draw_var_values`.
3. Canvas (`CentralPanel` of the pane): `draw_items(ui, cx, tile_id)`, gestures,
   markers, annotations, context menu — the existing code with `viewport_idx`
   replaced by the tile's `Viewport`.

The focus-id overlay (`focus id list`, shown while typing `item_focus`) is
drawn only in the target waveform tile. The transaction-details and annotation
list panels are gone from here (they are tiles).

### 6.5 Titles

Default titles: `Waveform` when there is exactly one waveform tile, otherwise
`Waveform 1`, `Waveform 2`, … numbered by position in `tile_order()`; linked
tiles show a link glyph and the list number (`Waveform 2 ⇄1`). `Memory: top.mem`,
`Markers`, `Logs`, `Annotations`, `Transaction`. `title: Some(..)` overrides.
Computed once per frame into `titles`.

---

## 7. User experience

### 7.1 First run and defaults

Fresh start: empty layout, splash in the central area ("Load a file… / Ctrl+O").
Loading a file: one waveform tile (tab bar hidden if
`hide_single_tab_bar = true`). Surfer looks and behaves exactly as today except
for a tab bar.

### 7.2 Creating

* **View ▸ New Tile ▸** {Waveform, Memory viewer…, Markers, Logs, Annotations,
  Frame buffer}. Inserted `Beside(focused, Right)` and focused.
* Tab bar **+** button: same list, plus Split Right / Split Down.
* Item context menu: "Show memory viewer" → typed memory creation beside the originating tile.
* Palette: `tile_new memory`, `show_logs`, …
* Toolbar group `viewports` is renamed `tiles`: Split Right, Split Down, Close.

### 7.3 Splitting

Split Right/Down on a waveform tile creates a **linked** tile (same items, own
zoom). Split Copy creates an independent copy of the item list. Other kinds
split if they implement `split_clone` (memory viewer: yes; logs/markers: no).
Split from keyboard uses the focused tile; from the tab menu, the clicked tab.

### 7.4 Moving and tabbing

Drag a tab and drop it: on the centre of another tile → becomes a tab there;
on an edge → new split on that side; on the tab bar → reorder. Keyboard:
`tile_move_*` moves the focused tile one step (swap with the neighbour in a
split, or split the group edge). Mouse and keyboard structural edits use the same transaction and undo policy;
scripts express moves through `MoveTile` and `Placement`.

### 7.5 Closing

Tab ✕, `Ctrl+W`, tab menu, `tile_close`. Closing the last tile leaves an empty
layout with the splash. Closing a waveform tile that is the last user of its
item list drops the list (undoable). No tile is privileged or unclosable.

### 7.6 Focus

Exactly one focused tile (or none). Focus follows clicks and tab selection; it
is shown by the tab's accent colour and a 1px frame (theme `tile_focus_stroke`).
Keyboard focus navigation: `Ctrl+Shift+Arrows` spatially, `Ctrl+PageUp/Down`
within a group. `target_waveform()` is remembered separately so that focusing a
non-waveform tile does not change which waveform reacts to zoom, markers or
"add variable".

Hierarchy sidebar → adds to `target_waveform()`. If none exists, a waveform tile
is created first. The overview strip shows every waveform tile's window and
highlights the target one; clicking a window in the overview focuses that tile.

### 7.7 Commands and the palette

The palette captures and resolves its target when opened (§5.1). Its suggestion
lists (items, markers, variables in scope) come from the target waveform. Kind-specific
commands (`memory_goto <index>`, future `table_sort`) are registered by the
kind and are only offered when the captured tile is of that kind: `get_parser`
asks `captured tile → view().commands()` (an optional trait method returning
`Vec<CommandSpec>`). This keeps `command_parser.rs` free of
per-kind knowledge.

### 7.8 State files

`Ctrl+S` writes the whole workspace. Loading a `.surf.ron` restores layout,
tiles and item lists and reattaches them to the loaded waves. The sibling
state file mechanism (`<wave>.surf.ron` next to the wave) is unchanged and now
restores the full layout.

---

## 8. Serialization

### 8.1 Format and versioning

Keep RON and the `.surf.ron` extension, but separate file DTOs from live runtime
structs. The file contract is explicit:

```rust
struct WorkspaceFile {
    version: u32,
    layout: LayoutFile,
    tiles: BTreeMap<TileId, TileFile>,
    item_lists: BTreeMap<ItemListId, ItemListFile>,
    document: Option<DocumentFile>, // source, shared cursor/markers/settings
    // existing persistent preferences …
}
struct TileFile {
    title: Option<String>,
    kind: String,                  // stable registry name, e.g. "waveform"
    kind_version: u32,
    payload: Box<ron::value::RawValue>,
}
```

Each kind owns typed payload DTOs and migrations. For example, the waveform
payload holds `items`, viewport, scroll, focus and columns; the memory payload
holds a stable array path and display settings. Runtime caches, adapter trees,
allocators, pending jobs and document handles never enter the file. Saving uses
stable map ordering and validated references. The generic `TileEntry` and
`TileKind` do not derive the workspace file format.

The workspace version selects the container schema; `kind_version` selects a
kind's payload schema. Missing version means legacy version 0. Unsupported
workspace versions are rejected clearly before replacing state. Unsupported
kind versions are preserved as unknown tiles. Defaults apply only to genuinely
optional fields with specified semantics; required IDs, layout structure and
payload fields are validated rather than silently defaulted.

### 8.2 Unknown kinds and errors

Dispatch a `TileFile` using `(kind, kind_version)`. For a supported pair, decode
the raw payload directly into that kind's DTO and validate it. Malformed known
payloads report a load error; they are not mislabeled as newer kinds. Unknown
pairs become placeholder tiles retaining the original kind, version, title and
raw payload. Save their original envelope, not an `Unknown(...)` enum variant.

Do not decode typed RON payloads through `ron::Value`: it does not retain enum
variants. Raw payload preservation retains unknown syntax; outer formatting
need not remain byte-identical. Add a fixture containing nested enum variants
and a future kind version to prove the actual codec round-trips them. See
[RON Value documentation](https://docs.rs/ron/latest/ron/value/enum.Value.html)
and [RawValue documentation](https://docs.rs/ron/latest/ron/value/struct.RawValue.html).

A kind payload must not be the sole owner of references into generic workspace
storage that older builds need to garbage-collect. In version 1 only waveform
payloads reference `item_lists`; if an unknown kind/version exists, conservatively
retain all loaded lists, allow unowned retained lists during validation, and
skip automatic list collection until those unknown entries are removed. Future
shared resource types require explicit envelope-level dependencies or a new
workspace version. This prevents a save through an older build from destroying
resources needed by an unknown tile. Unknown tiles may be moved, renamed or
explicitly closed, but cannot split-clone or execute kind commands.

### 8.3 Legacy migration (`version 0` files)

Decode legacy files with a separate `LegacyUserStateV0` DTO using the original
field names. Do not keep `legacy_*` mirrors on the live `WaveData`; that mixes
migration concerns into every runtime operation. Migration is a pure
`LegacyUserStateV0 -> WorkspaceFileV1` conversion followed by normal validation.

1. Build one item list from the old item tree, displayed items, annotations and
   graphics, even if the tree is empty. Translate UI-derived IDs to stable domain
   IDs where needed; transient menus and selections may reset explicitly.
2. Build one linked waveform tile for each saved viewport; if a loaded document
   has no viewports, create one default waveform tile. Preserve saved viewports,
   focus and scroll where valid. Use horizontal splits and expose columns on
   the first tile only. The linked-scroll option in §14 is required to preserve
   row alignment for this legacy shared-column layout.
3. Translate persisted widget visibility flags into corresponding tile entries.
   Keep generic preferences and shared cursor/marker times. Invalid historical
   focus indices fall back to the first waveform; document this repair.
4. Produce version 1, validate the complete result, then install atomically.

Legacy `.sucl` command aliases are handled by the input parser, not by state-file
migration. Retain the legacy decoder while legacy files remain supported; its
lifetime is independent of internal struct evolution.

A pending state file is a validated `PendingWorkspace`, not a recursive boxed
`UserState`. It carries the file DTO and a load-request token. Only the matching
wave-load completion may attach its document and install it; an older load
completion cannot replace a newer user request. Do not partially replace the
active workspace on decoding, validation or document-load failure.

---

## 9. Undo / redo

Undo records describe successful semantic operations. Do not clone the whole
workspace for every item edit. Persistence and undo have different boundaries:
zoom, scroll, focus, tab activation, column widths and split resizing persist
in files but do not get their own undo entries or roll back during unrelated
undo operations.

Use a small explicit `UndoRecord` enum with before/after data for affected
content and structural inverses:

* Item-list edits retain changed list content (whole affected-list snapshots
  initially are acceptable). Shared lists are captured once. Runtime caches
  and per-tile view settings are excluded.
* Kind-setting edits (array selection, filters, formats) retain only changed
  semantic settings through a kind-owned undo payload; unrelated navigation is
  preserved. Marker-time edits retain changed shared marker values. Annotation edits
  retain list content; selection is sanitized after restoration.
* Create/close/split retain affected tile entries and any owned list content
  required to recreate them. Reopened tiles restore their saved view settings;
  surviving tiles retain their current zoom, scroll and other view settings.
* Move/dock/reorder retain the moved tile and its old/new parent placement,
  with stable neighboring tile anchors. Generic rename retains old/new title.
  Mouse and keyboard operations produce the same records.

Structural records store the affected topology and necessary insertion shares,
not a full view-state snapshot. Preserve current shares for surviving splits;
restore recorded shares only when reconstructing removed containers. Undoing a
split necessarily changes geometry, but unrelated resizing is retained. Undo
restores visibility and repairs focus only if the current focus no longer
exists; it does not restore historical focus wholesale. Retain enough ancestor
placement context to recreate a removed group; tests cover moves that collapse
and recreate nested splits.

Multi-step commands and one completed drag gesture form one history entry.
Failed/no-op commands create none. Followups remain part of the originating
transaction. `SetLayout` is an explicit whole-topology replacement with a
corresponding topology record, preserving surviving tile view settings.

New semantic edits clear redo; ordinary navigation does not. Workspace install,
document replacement/reload and legacy migration clear undo/redo as an explicit
boundary, since old records contain references to the previous attachment.
Async cache completions never enter history. Restoring a tile/list clears its
runtime caches and pending requests; fresh work receives fresh request tokens.
Session allocators and generation counters never roll back.

Keep `undo_stack_size`, and measure memory with multiple independent lists
before optimizing snapshots. Use shared immutable content or more granular
records only if measurements justify the complexity.

---

## 10. Adding a new tile kind

A new kind requires:

1. A state struct, typed payload DTO/codec with version, local command enum,
   optional internal completion enum, and `TileView` implementation in its module.
   Rendering is immutable except for runtime scratch/cache state.
2. Registry entries for enum dispatch, codec, factory and capabilities. The
   factory receives the context needed to create valid state; the common entry
   owns the title. Kind-specific palette `CommandSpec`s live with the kind.
3. Round-trip, command/targeting and snapshot tests, plus documented behavior
   for missing data, reload and stale async completions.
4. Any desired context-menu entry point, producing a typed creation request.

No per-kind branches belong in layout/rendering/undo dispatch. A kind with new
editable content supplies the relevant undo payload through the transaction
contract; registry dispatch delegates to it. New kinds that need shared resources
must also define their persistence ownership contract (§8.2).

### 10.1 Worked example: the memory viewer

`MemoryTile` owns a stable array path, index/value formats, column count, search,
highlight and filter settings. The generic entry owns its optional title.
Runtime state holds a cache and request token, never persisted backend handles.

`MemoryMessage` covers array selection, formats, column count, filters and row
navigation. `ui` emits `ToTile(its_id, TileMessage::Memory(...))`; it does not
mutate settings or resolve ambient focus. The context-menu entry point creates
a memory tile with the selected array path through the registry factory.

The cache key includes workspace/document generation, array identity, cursor,
and every setting that affects cached values or rows. Changing these inputs
invalidates pending work. A missing array/cursor renders an explanatory empty
state while preserving the tile settings; reload reattaches the stable path.
Split-clone copies settings and starts with empty runtime state. Selection and
scroll are view navigation, not edits to the immutable trace.

Remove `MemoryViewerState::open`, the singleton `SystemState::memory_viewer`
state/cache, `Message::OpenMemoryViewer` and the old floating window when this
kind is migrated. Its existing rendering code moves into the kind module.

### 10.2 Sketch: a signal change table

A future `SignalTableTile` owns stable variable references, sorting, filters
and column keys. It renders rows from immutable off-thread snapshots, using
`SignalAccessor` data where appropriate. The request key includes the complete
set of model inputs, including document generation, translator/config revision,
variables, sorting and filtering if the model applies them.

Submit build requests through the dispatcher and receive internal
`TileCompletion::SignalTable` results with the tokens from §4.1. Validate tokens
and input keys before accepting the result; never inject async results through
deserializable user commands. Track pending/ready/failed explicitly. Failures
end readiness waits and render an error with retry; hidden tiles do not start
unnecessary work, and tests wait only for work their scenario requires.

Row activation sends a shared cursor command. Highlighting follows the shared
cursor. The table subsystem is separate work; no changes to layout ownership
or targeting are needed to add it.

---

## 11. Migration path

Ordered so that each step compiles, passes tests and could ship.

### 11.1 Step 1 — Extract content and input boundaries

Extract `ItemList` without UI changes; temporarily keep one list on `WaveData`.
Separate item-list content from view navigation and runtime caches. Introduce
typed waveform commands and explicit target resolution; adapt the existing
single-waveform view before adding multiple tile identities. Update affected
callers and tests together, without retaining a second internal command path.

### 11.2 Step 2 — Prove the layout adapter and file contract

Pin the `egui_tiles` release compatible with the workspace's egui version
(proposed 0.17 / egui 0.36; verify dependency/API compatibility before coding).
Build small adapter tests for docking, tab selection, resizing, stable IDs,
clipping and gesture boundaries. Prove `LayoutNode` conversion/validation,
unknown raw payload preservation and legacy migration with fixtures. Do not
build the larger migration on unverified callback or codec assumptions.

### 11.3 Step 3 — Install the tile-native workspace

Add `TileEntry`, session allocators/epochs, concrete-target commands, validated
workspace transactions and undo records. Move waveform rendering into its kind;
install per-tile draw caches and content-space list layout caches. Convert the
existing viewport tests to linked-tile tests, including aligned scrolling for
the legacy shared-column mode. Switch persistence to DTOs and wire pending-load
tokens and atomic workspace installation.

On document load/reload, preserve the layout and kind settings by default:

* A fresh workspace with a document gets one empty waveform/list if none exists.
* A different file clears old item content under `LoadOptions::Clear`, resets
  document-relative navigation, and marks invalid non-waveform targets
  unavailable. Keep tile arrangement and presentation preferences. Clear shared
  cursor/markers that belong to the old document.
* `KeepAvailable`/`KeepAll` reattaches every list with the corresponding
  unavailable-item policy, clamps/reset viewports as appropriate, sanitizes tile
  focus references, and calls every kind's document-change hook.
* A validated pending state installs its saved layout/settings only when its
  matching document load succeeds, followed by the same attachment validation.
* Explicit **Reset Workspace** restores the default layout; it is separate from
  opening a file. A sibling state file explicitly replaces the layout when applied.

Every document change advances generation, clears pending jobs and undo/redo,
and invalidates dependent caches. Missing references are visible unavailable
states rather than silently disappearing tiles.

### 11.4 Step 4 — Migrate widgets

Move memory, markers, logs, frame buffer, annotations and transaction details
one kind at a time. Delete each replaced singleton field/visibility flag and
floating window in the same change. Validate that context-menu, keyboard and
palette entry points use identical targeting and creation semantics.

### 11.5 Step 5 — Interaction and external interfaces

Complete focus navigation, drag from hierarchy, overview targeting, per-kind
palette commands and WCP compatibility adapters. Document captured targeting
and the limits of WCP's legacy per-list item references. Add optional explicit
tile/list addressing only through a coordinated protocol change. Update user
commands and state-format documentation alongside implementation.

### 11.6 Step 6 — New kinds

Signal tables and pipeline/event views can then use the tested contract in §10.

---

## 12. Testing

* **Layout:** normalization and tree round trips; duplicate/missing tile IDs,
  missing lists, invalid shares/tab indices, depth limits, focus visibility and
  singleton enforcement. Failed commands leave state unchanged.
* **Targeting:** interacting with an unfocused tile edits its own list; palette
  target stays fixed while focus changes; closed targets never redirect;
  followups retain targets; script statements resolve sequentially.
* **Undo:** edit → zoom → undo preserves zoom; close/reopen restores owned
  resources; shared lists are restored once; dock/reorder and keyboard moves
  have identical history; unrelated resizing survives undo; collapsed nested
  groups reconstruct correctly; workspace/document changes clear history.
* **Persistence:** empty and populated legacy files; legacy files with no
  viewports; normal workspace round trips; unknown kind/version payloads with
  nested enums and retained resources; malformed known payload errors;
  unsupported workspace versions; invalid input never partially installs.
* **Async:** completions after close, undo restoration, array/filter change,
  document reload and workspace load with reused numeric IDs are rejected;
  failure/cancellation ends pending state; overlapping file loads obey tokens.
* **Snapshot:** linked versus independent lists, legacy row alignment, hidden
  columns, focus/empty states, every kind and unavailable data, tab bar modes.
  Wait for required tile work with a bounded timeout and diagnostic failures.
* **Interaction:** real-input tab focus/close, docking, reorder, split resizing,
  drag cancellation and cross-tile drops. Test Surfer's command/undo integration
  even though the third-party library implements the drag mechanics.
* **External:** legacy command aliases and WCP viewport ordering/target capture;
  injected malformed commands cannot bypass validation.

---

## 13. Lessons taken from the PoC

Kept: `egui_tiles` (≈500 lines of glue gave splits, tabs, docking and
resizing); serialized "spec + view config" per tile with runtime state
elsewhere; off-thread model building with generation-keyed caches and
`is_ready` gating for tests; key-based column identity; models pushing time via
`CursorSet` rather than reaching into app state.

Avoided: a separate non-tiled code path for the single-tile case; per-kind
`HashMap`s in `UserState`/`SystemState` and per-kind `mem::take` in the draw
pass; enum dispatch spread over ~12 files per kind; an "active tile" that only
exists for one kind and is set from inside draw code; serialized state mutated
during draw; `#[serde(default)]` id counters; resetting the tree on load while
leaking tile state; egui id collisions from global panel names; a privileged
unclosable waveform pane; bundling the layout with unrelated multi-source work.

---

## 14. Decisions and remaining scope

1. **Keep linked item lists.** They preserve shared signal sets. Add explicit
   vertical-scroll linking for the legacy shared-column mode: a runtime gesture
   updates all opted-in waveform views with the same persisted `ItemListId`. Default migrated extra viewports to this mode. Ordinary
   linked splits may scroll independently; labels/menu text must distinguish
   shared items from shared scroll. Do not claim parity until alignment is tested.
2. **One shared cursor initially.** Independent secondary cursors and horizontal
   zoom-sync groups are future features with explicit ownership and target rules.
3. **Keep hierarchy and global chrome outside the tree.** Hierarchy additions
   use the declared waveform fallback policy; kind-local commands target focus.
4. **Persist navigation, exclude it from unrelated undo.** Structural edits,
   including mouse docking/reordering, are undoable; resizing and focus are not.
5. **Preserve arrangement across document changes.** Missing targets render
   unavailable states. Reset is explicit; applying a saved workspace replaces it.
6. **WCP compatibility stays at the boundary.** Positional viewport addressing
   maps to current waveform order. Multi-list client identity needs a future
   explicit protocol extension, not implicit focus-dependent references.
7. **Show single-tile tab bars by default.** Keep a hide option. Browser shortcut
   bindings need platform testing; the palette and close button remain available.
8. **No plugin system or multi-document abstraction yet.** An enum registry,
   one document, explicit commands and versioned per-kind DTOs are sufficient.
   Validate performance and API ergonomics before adding further abstraction.

---

## 15. Implementation status

This section records how the shipped code maps onto the design above and
where it deliberately deviates. It is the reference for extending the
workspace; earlier sections describe intent, this one describes the contract
that exists.

### 15.1 Module map

| Design | Code |
|---|---|
| `tiles/mod.rs` identity, `TileTarget` | `tiles/mod.rs` |
| `tiles/layout.rs` | `tiles/layout.rs` (`Layout`, `LayoutNode`, `Placement`, validation, geometry, spatial navigation) plus `tiles/placement.rs` (stable anchors for move undo) |
| `tiles/kind.rs` registry | `tiles/kind.rs`: `TileKind`, `TileMessage`, `TileSettings` (undo payloads), `KindDescriptor`/`KINDS`, codecs, `KindCommand` palette registry, `ApplicationPanes` renderer |
| `tiles/view.rs` trait + `TileCtx` | `tiles/view.rs`: `TileCtx`/`TileReadServices`, generic tab context menu. There is no `TileView` trait object; kinds dispatch through the enum (§15.2) |
| `tiles/render.rs` | `tiles/render.rs`: `egui_tiles` adapter, `PaneRenderer`, proposals, focus/close events, tab bar `+` menu |
| `tiles/serde.rs` | `tiles/serde.rs` (envelopes, raw payloads, list DTO) and `tiles/workspace.rs` (`WorkspaceFile`, validation, atomic `replace`) |
| `tiles/commands.rs` transactions/undo | `tiles/commands.rs` (`WorkspaceCommand`, `DocumentCommand`), `tiles/workspace.rs` (dispatcher), `tiles/history.rs` (`UndoRecord`) |
| `tiles/runtime.rs` | `tiles/runtime.rs` (allocators, epochs, request tokens) |
| input resolution (§5) | `tiles/input.rs`: `CommandTarget`, and `Workspace` helpers that turn keyboard/palette/toolbar/menu intent into concrete commands |
| legacy migration (§8.3) | `tiles/legacy.rs`: `LegacyWaveDataV0` DTO → `LegacyWaveformV0::into_workspace`; fixture `tiles/fixtures/legacy-state-v0.ron` |

### 15.2 Deviations from the text above

* **Enum dispatch instead of a `TileView` trait object.** `TileKind` matches
  exhaustively in `kind.rs` for rendering, update, codecs, runtime reset,
  split policy and palette commands. Adding a kind touches that one file plus
  the kind's module; the compiler lists every dispatch point.
* **Kind-specific palette commands are static tables.** `KindCommand { name,
  suggestions, parse }` is registered per kind (`tile_columns`,
  `tile_link_scroll`, `logs_filter`, `annotation_list_comments`). The parser
  offers them only while the captured tile is of that kind.
* **Captured palette target.** `CommandPrompt::target` is captured when the
  prompt opens and used for suggestions and execution; `ExecuteBatchCommand`
  resolves each statement against the live workspace.
* **Marker history.** `UndoRecord::Marker { id, time, lists }` retains one
  marker's previous time and the affected lists; item-edit records never
  snapshot shared marker times.
* **`WorkspaceCommand::Reset { keep }`** implements "Reset Workspace"
  (§11.3): keep the target waveform or create an empty one, as one undoable
  record. Palette `workspace_reset`, Tiles menu entry.
* **Legacy `viewport_*` spellings** and `Message::AddViewport`-style variants
  are gone from the internal API; the parser maps `viewport_add`,
  `viewport_remove` and `viewport_set_active` to tile commands on the target
  waveform.
* **Titles** are computed once per frame by `Workspace::titles()`:
  `Waveform` / `Waveform N`, a link glyph plus list number for views sharing a
  list, `Memory: <name>` for memory tiles, entry title overrides.
* **Toolbar group `tiles`** replaces `viewports` (Split right, Split down,
  Close). Menus and the tab bar `+` use the same resolvers as the palette.
* **Document loads are staged.** A parsed header waits in
  `SystemState::pending_document` until its body arrives; every load carries
  a request id and stale or failed completions never touch the workspace.
* **Async tile work.** No kind performs off-thread work yet, so
  `WorkspaceRuntime::request`/`accepts` are the token contract for future kinds
  (§10.2) and are covered by unit tests only.

### 15.3 Adding a kind, concretely

1. Add the state struct, its serde DTO with `#[serde(deny_unknown_fields)]`,
   its message enum and (if it edits content) its `TileSettings` payload in
   `tile_kinds/<kind>.rs`.
2. Register in `tiles/kind.rs`: `KindDescriptor`, `TileKind` variant,
   `TileMessage` variant, `TileEntry::{from_file,to_file}`, `create`,
   `descriptor`, `default_title`, `supports_split`/`split_clone`,
   `reset_runtime`, `apply_tile_message`, `ApplicationPanes::ui`, and
   `commands()` when it offers palette commands.
3. Add an opener (`Workspace::open_command` covers the generic case), tests
   for round trip, targeting and undo, and a snapshot showing the empty and
   unavailable states.
