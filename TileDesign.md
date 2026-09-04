# Tiles & Tabs in Surfer — Design

Status: proposal, to be implemented on the `vtr` branch.
Baseline: `db1ca915` (egui 0.36.1, y-location cache rework included).
Reference: `origin/table-ftr-event-vibes` (PoC, egui 0.35, `egui_tiles` 0.16) — ideas only, not a baseline.

---

## 1. Summary

Surfer gets a VSCode-like workspace: the central area is a tree of **splits** and
**tab groups** whose leaves are **tiles**. A tile is one presentation of the
loaded (immutable) data: a waveform view, a memory viewer, a marker table, a log
panel, later signal tables and pipeline views. Tiles can be created, split,
dragged into other groups, tabbed, closed, and focused with mouse or keyboard.
Commands and the palette act on the focused tile. The whole layout round-trips
through the `.surf.ron` state file.

The design in one paragraph:

* `UserState` becomes tile-native: `layout` (the tree), `tiles` (tile state by
  id), `item_lists` (what waveform tiles display, shareable between tiles), and
  `waves` (the shared document: data container, cursor, markers, time range).
* The layout engine is `egui_tiles` 0.17 at runtime; Surfer owns the
  serialized layout format and the tile identity, so the file format does not
  depend on the crate.
* A tile kind is a struct implementing one trait (`TileView`) plus one variant
  in a registry enum (`TileKind`). Its state, rendering, messages and
  serialization live in its own module. The layout core never matches on a
  concrete kind.
* All state mutation stays message-driven. Tile `ui` takes `&self`.
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

Assets to reuse unchanged: `Viewport` (`Copy`, serde, relative time),
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
├── tiles: BTreeMap<TileId, TileKind>
│     ├── 1 → Waveform(WaveformTile { items: ItemListId(1), viewport, scroll, focus… })
│     ├── 2 → Waveform(WaveformTile { items: ItemListId(1), viewport, … })   ← linked
│     ├── 3 → Waveform(WaveformTile { items: ItemListId(2), … })             ← independent
│     ├── 4 → Memory(MemoryTile { scope, name, formats, filters… })
│     └── 5 → Markers(MarkersTile {})
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
  async work. Lives in `#[serde(skip)]` fields on the owning struct.

---

## 4. Core types

Module layout:

```
libsurfer/src/tiles/
├── mod.rs        re-exports; TileId, ItemListId, TileTarget
├── layout.rs     Layout, LayoutNode, split/tabs/move/close operations, focus navigation
├── kind.rs       TileKind + TileMessage registry (the only place that lists all kinds)
├── view.rs       TileView trait, TileCtx
├── render.rs     egui_tiles::Behavior impl, draw_layout, tab bar chrome, focus detection
└── serde.rs      Layout <-> LayoutNode conversion, legacy migration
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

Allocation: `UserState` keeps `#[serde(skip)] next_tile_id: u64` and
`next_item_list_id: u64`. Both are **recomputed on load** as `max(keys) + 1`,
exactly like `display_item_ref_counter` today (`wave_data.rs:293-298`). This
avoids the PoC bug where `#[serde(default)]` counters restarted at 0 and
aliased existing tiles.

`egui_tiles::TileId` (the crate's node id) is a runtime detail and is never
serialized or exposed in messages. Panes in the runtime tree carry our `TileId`.

### 4.2 Layout

```rust
pub struct Layout {
    /// Runtime layout engine. Rebuilt from `LayoutNode` on load.
    #[serde(skip)]
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

`Layout` implements `Serialize`/`Deserialize` by hand via a private
`LayoutFile { root: Option<LayoutNode>, focused, focus_history }`. Conversion
`tree -> LayoutNode` walks `egui_tiles::Tiles`:
`Container::Linear` → `Split` (shares taken from `Linear::shares` in child
order), `Container::Tabs` → `Tabs` (`active` as index), `Container::Grid` →
`Split { Horizontal }` (grids are never created by Surfer; tolerated on read).
`LayoutNode -> tree` inserts panes and containers and sets shares. Both
directions are pure functions with unit tests (round trip, and "every `Tile`
id exists in `tiles`").

Operations (all on `Layout`, all pure tree edits, used by message handlers):

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
    pub fn rect(&self, tile: TileId) -> Option<Rect>;      // last frame's rect, for hit tests
    pub fn simplify(&mut self);                            // prune empty/single-child containers
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
    /// Empty layout or no anchor: become the root.
    Root,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Left, Right, Up, Down }
```

The tree is allowed to be empty (all tiles closed). The central area then
shows the welcome/splash content with a hint; loading a file into an empty
layout creates one waveform tile.

### 4.3 Tile kinds: the registry and the trait

`tiles/kind.rs` is the **only** file that enumerates kinds:

```rust
#[derive(Serialize, Deserialize, Clone)]
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
            (Self::Memory(t), TileMessage::Memory(m)) => t.update(m, cx),
            (Self::Markers(t), TileMessage::Markers(m)) => t.update(m, cx),
            /* … */
            (t, m) => warn!("{} tile ignored {m:?}", t.kind_name()),
        }
    }
    /// For menus, the palette and `tile_new <kind>`.
    pub const CREATABLE: &[(&str, fn() -> TileKind)] = &[
        ("waveform", || TileKind::Waveform(WaveformTile::default())),
        ("memory", || TileKind::Memory(MemoryTile::default())),
        ("markers", || TileKind::Markers(MarkersTile::default())),
        ("logs", || TileKind::Logs(LogsTile::default())),
        /* … */
    ];
}
```

Four match sites in one file, plus the enum variant. A `tile_kinds!` macro can
generate them if the list grows; not required initially.

Why an enum and not `Box<dyn TileView>` with a deserialization registry:
serde derives handle the file format and messages for free, exhaustiveness
checks catch missing arms, and it works on wasm32 (where `typetag`/`inventory`
style registries are unreliable). The cost — one variant plus four one-line
arms per kind — is small and local.

Why waveform-tile messages are **not** in `TileMessage`: the waveform tile has
~100 existing top-level `Message` variants (items, zoom, markers…). Moving them
would churn most of `update` for no gain. They stay top-level and act on the
**target waveform tile** (§5.1). New kinds use `Message::ToTile`.

The trait each kind implements (`tiles/view.rs`):

```rust
pub trait TileView {
    /// Stable kind name: menus, palette, tab default title, docs.
    fn kind_name(&self) -> &'static str;

    /// Tab title. Default: kind name; a tile with `title: Some(..)` overrides.
    fn title(&self, cx: &TileCtx) -> String;

    /// Draw the tile body. Immutable: all changes are sent as messages via `cx`.
    /// Per-frame scratch state lives in egui memory or `#[serde(skip)] RefCell` fields.
    fn ui(&self, ui: &mut egui::Ui, cx: &mut TileCtx);

    /// A copy suitable for "split": `None` means the kind cannot be split-cloned
    /// and the split menu entry is disabled for it.
    fn split_clone(&self) -> Option<TileKind> { None }

    /// Only one instance makes sense (logs, markers): "open" focuses the existing tile.
    fn singleton(&self) -> bool { false }

    /// Called after the shared document changed (reload, switch_file, new file).
    /// Fix up references or degrade gracefully; return false to close the tile.
    fn on_waves_changed(&mut self, change: WavesChange, cx: &mut TileUpdateCtx) -> bool { true }

    /// Extra entries for the tab context menu (after the generic ones).
    fn tab_context_menu(&self, ui: &mut egui::Ui, cx: &mut TileCtx) {}

    /// For tests: false while async work (caches) is pending.
    fn is_ready(&self, cx: &TileCtx) -> bool { true }
}

pub enum WavesChange { Loaded, Reloaded { keep_unavailable: bool }, Cleared }

pub struct TileCtx<'a> {
    pub app: &'a SystemState,
    pub tile_id: TileId,
    pub focused: bool,
    pub msgs: &'a mut Vec<Message>,
}
impl TileCtx<'_> {
    pub fn waves(&self) -> Option<&WaveData>;
    pub fn config(&self) -> &SurferConfig;
    pub fn theme(&self) -> &SurferTheme;
    pub fn translators(&self) -> &TranslatorList;
    pub fn send(&mut self, m: Message);
    /// Shorthand for `Message::ToTile(TileTarget::Id(self.tile_id), m)`.
    pub fn send_self(&mut self, m: TileMessage);
    /// Salted egui id for widgets inside this tile.
    pub fn id(&self, salt: impl Hash) -> egui::Id;
}

pub struct TileUpdateCtx<'a> {
    pub tile_id: TileId,
    pub waves: Option<&'a mut WaveData>,
    pub item_lists: &'a mut BTreeMap<ItemListId, ItemList>,
    pub config: &'a SurferConfig,
    pub translators: &'a TranslatorList,
    /// Messages to run after this update (e.g. invalidate caches).
    pub followups: &'a mut Vec<Message>,
}
```

`ui` takes `&self` on purpose: it is what the rest of Surfer already does
(widgets push messages, `update` mutates), it makes the borrow story trivial
(the tile pass borrows `SystemState` immutably, §6.1), and it keeps undo and
snapshot tests deterministic. The PoC's Konata tile mutated serialized state
during draw and paid for it.

### 4.4 Shared document: `WaveData` after the split

`WaveData` keeps:

```rust
pub struct WaveData {
    #[serde(skip, default = "DataContainer::__new_empty")]
    pub inner: DataContainer,
    pub source: WaveSource,
    pub format: WaveFormat,
    pub active_scope: Option<ScopeType>,     // hierarchy sidebar selection
    pub cursor: Option<BigInt>,              // shared across all tiles
    pub markers: HashMap<u8, BigInt>,        // marker times; rows live in item lists
    pub display_variable_indices: bool,
    // runtime
    #[serde(skip)] pub old_max_timestamp: Option<BigInt>,
    #[serde(skip)] pub cache_generation: u64,
    #[serde(skip)] pub inflight_caches: HashMap<AnalogCacheKey, Arc<AnalogCacheEntry>>,
    #[serde(skip)] pub cached_time_range: TimeRange,
}
```

Removed from `WaveData` and moved: `items_tree`, `displayed_items`,
`display_item_ref_counter`, `default_variable_name_type`, `annotations`,
`annotation_groups`, `annotation_counter`, `annotation_list_visible`,
`selected_annotation`, `annotation_menu_*`, `graphics`, `drawing_infos`,
`drawing_infos_signature`, `total_height` → `ItemList`;
`viewports`, `last_active_viewport_idx`, `scroll_offset`, `focused_item`,
`focused_transaction` → `WaveformTile`.

`WaveData::update_with_waves` (reload/switch) keeps only shared fields; the
per-list and per-tile reattachment is driven from `SystemState::on_waves_loaded`
(§11.3).

### 4.5 Item list

```rust
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ItemList {
    pub items_tree: DisplayedItemTree,
    pub displayed_items: HashMap<DisplayedItemRef, DisplayedItem>,
    pub ref_counter: usize,
    pub default_variable_name_type: VariableNameType,
    // anchored to items, so they belong here
    pub annotations: Vec<Annotation>,
    pub annotation_groups: Vec<AnnotationGroup>,
    pub annotation_counter: i32,
    pub selected_annotation: Option<egui::Id>,
    pub graphics: HashMap<GraphicId, Graphic>,
    // runtime: y-location cache (was WaveData::drawing_infos)
    #[serde(skip)]
    pub layout_cache: RefCell<ItemLayoutCache>,   // { signature: u64, infos: Vec<ItemDrawingInfo>, total_height: f32 }
}
```

Everything that today takes `&WaveData` to read items takes `&ItemList`
instead (`add_variables`, `remove_items`, `move_item`, `compute_variable_display_names`,
`update_with_items`, `visible_drawing_infos`, …). This is a mechanical move; the
methods' bodies do not change. `DisplayedItemRef` and `VisibleItemIndex` are
scoped to a list; a `(ItemListId, DisplayedItemRef)` pair is only needed at the
WCP boundary (§5.6).

An item list with no referencing tile is dropped when the last tile closes
(after the undo snapshot is taken, §9).

### 4.6 Waveform tile

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct WaveformTile {
    pub items: ItemListId,
    pub viewport: Viewport,                       // zoom/pan; was WaveData::viewports[i]
    pub scroll_offset: f32,
    pub focused_item: Option<VisibleItemIndex>,
    pub focused_transaction: (Option<TransactionRef>, Option<Transaction>),
    pub show_name_column: bool,                   // default true
    pub show_value_column: bool,                  // default true
    pub title: Option<String>,                    // user rename; None = "Waveform" / "Waveform N"
    // runtime
    #[serde(skip)] draw_cache: RefCell<Option<WaveDrawCache>>,   // { canvas_rect, viewport, list_signature, data: CachedDrawData }
}
```

One waveform tile = one canvas plus its own name and value columns. Today's
"add viewport" (extra time axis sharing the name column) is expressed as a
**linked split**: a second waveform tile referencing the same `ItemListId`,
with `show_name_column = false` on the right-hand tile if the user wants the
old look. Two tiles on the same list show the same items, folds, selection and
markers; each has its own zoom, scroll and focused item. Editing the list from
either tile is visible in both, which is exactly what the old viewports did.

The draw cache is keyed by (canvas rect, viewport, item-list signature) and
lives in the tile. `invalidate_draw_commands()` becomes:

* `invalidate_tile(id)` — after zoom/pan/resize of one tile;
* `invalidate_list(list_id)` — after an item edit: every tile whose `items == list_id`;
* `invalidate_all()` — reload, config, theme.

This fixes the every-frame regeneration caused by the shared `last_canvas_rect`.

### 4.7 Migrated widget kinds

| Kind | State (serialized) | Reads (shared) | Notes |
|---|---|---|---|
| `MemoryTile` | `scope: ScopeRef`, `name: String`, formats, filter/search/highlight settings, `value_column_count`, `color_values` (all of `MemoryViewerState` minus `open`/scroll/selection); runtime: `RefCell<Option<MemoryViewerCache>>` | `waves.cursor`, `waves.inner` | Multiple instances allowed (different arrays). "Show Memory Viewer" in the item context menu opens one beside the focused tile. `on_waves_changed` → keep if the array still exists, else close. |
| `MarkersTile` | none | `waves.cursor`, `waves.markers`, marker rows of the target waveform's list | Singleton. Replaces `show_cursor_window`. |
| `LogsTile` | `filter: LevelFilter` | global log buffer | Singleton. "Open on error" becomes `Message::OpenTile { kind: logs, placement: Edge(Down), focus: false }`. |
| `FrameBufferTile` | `FrameBufferSettings` (from `UserState::frame_buffer`) + which variable/array | `waves.cursor` | Replaces the `frame_buffer_content` window. |
| `AnnotationListTile` | `show_comments: bool` | annotations of the target waveform's list | Singleton. Replaces `show_annotation_list` right panel. |
| `TransactionDetailsTile` | none | `focused_transaction` of the target waveform tile | Singleton. Today it appears automatically when a transaction is focused; new behaviour: focusing a transaction opens it (once) at `Edge(Right)` if not open. |

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

```rust
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum TileTarget { Id(TileId), Focused }
```

Resolution in `SystemState`:

```rust
/// The tile that ambient commands act on.
fn focused_tile(&self) -> Option<TileId>;
/// The waveform tile that item/zoom/marker commands act on: the focused tile if
/// it is a waveform, else the most recently focused waveform tile.
fn target_waveform(&self) -> Option<TileId>;
fn resolve(&self, t: TileTarget) -> Option<TileId>;
fn resolve_waveform(&self, t: TileTarget) -> Option<TileId>;   // Focused → target_waveform()
```

`target_waveform` is the analogue of VSCode's "active editor group": with the
logs tile focused, `+` still zooms the last-used waveform. When no waveform
tile exists (all closed, or only non-waveform tiles), commands that need one
create it: `AddVariables` from the hierarchy inserts a waveform tile at
`Placement::Root` or `Beside(focused, Right)` and targets it.

Rule for existing messages: a message that does not carry a `TileTarget` acts
on `target_waveform()`. A message that carried `viewport_idx: usize` now
carries `tile: TileTarget`. Producers that know the tile (canvas interaction,
tab menus, drag-and-drop) send `Id`; keyboard, toolbar, menu and palette send
`Focused`.

### 5.2 Message changes

Replaced (`viewport_idx: usize` → `tile: TileTarget`):
`CanvasScroll`, `CanvasZoom`, `ZoomToCursor`, `ZoomToRange`, `ZoomToFit`,
`GoToStart`, `GoToEnd`, `GoToTime(Option<BigInt>, TileTarget)`,
`GoToMarkerPosition(u8, TileTarget)`, `GoToAnnotationPosition(Id, TileTarget)`,
`AnnotationClicked(.., Option<TileId>, ..)`, `SetViewportStrategy` (applies to
all waveform tiles, unchanged semantics).

Removed: `AddViewport`, `RemoveViewport`, `SetActiveViewport`, `SetLogsVisible`,
`SetCursorWindowVisible`, `ToggleAnnotationlistVisibility`, `OpenMemoryViewer`,
`SetFrameBufferVisibleVariable` and the `show_*` fields behind them. Their
commands stay as aliases (§5.3).

New layout messages (top-level, flat, like the rest of `Message`):

```rust
/// Create a tile. `focus` false is used by "open logs on error".
AddTile { kind: TileKind, placement: Placement, focus: bool },
/// Open a singleton kind: focus the existing tile if any, else AddTile.
OpenTile { kind: TileKind, placement: Placement, focus: bool },
CloseTile(TileTarget),
CloseOtherTiles(TileTarget),          // in the same tab group
FocusTile(TileId),
FocusTileDirection(Direction),        // spatial neighbour
FocusTabDelta(isize),                 // next/previous tab in the focused group
/// Split: `linked` = share the item list (waveform only); otherwise `split_clone()`.
SplitTile { target: TileTarget, dir: Direction, linked: bool },
MoveTile { tile: TileId, to: Placement },
MoveTileDirection(TileTarget, Direction),   // keyboard move: swap with neighbour or split edge
RenameTile(TileTarget, Option<String>),
SetTileColumns { target: TileTarget, names: Option<bool>, values: Option<bool> },
/// Replace the whole layout; tests and WCP. Tiles referenced must exist.
SetLayout(LayoutNode),
/// Kind-specific.
ToTile(TileTarget, TileMessage),
```

Handling `CloseTile`: take undo snapshot; remove from layout; remove from
`tiles`; if the tile was a waveform and no other tile references its list,
remove the list; update `focused` to the next tab in the group, else the
spatial neighbour, else `None`; `layout.simplify()`; drop runtime caches.

Handling `SplitTile { linked: true }` on a waveform tile: clone the tile struct
(same `items`, same `viewport`, same scroll), insert `Beside(target, dir)`,
focus it. `linked: false`: deep-copy the item list under a new `ItemListId`
(annotations and graphics included), then as above. For other kinds:
`split_clone()` or refuse with a log message.

`Message` stays `Deserialize` (needed by `wasm_api::inject_message` and
`ExecuteBatchCommand`); `TileKind` and `Placement` derive it.

### 5.3 Commands (palette and `.sucl`)

New commands, all available without a loaded file:

| Command | Message |
|---|---|
| `tile_new <kind>` | `AddTile { kind, placement: Beside(focused, Right) }` |
| `tile_split_right`, `tile_split_down` | `SplitTile { Focused, Right/Down, linked: true }` |
| `tile_split_copy_right`, `tile_split_copy_down` | `SplitTile { …, linked: false }` |
| `tile_close`, `tile_close_others` | `CloseTile(Focused)` … |
| `tile_focus <id\|title>` | `FocusTile(id)` (suggestions: `"{id} {title}"`) |
| `tile_focus_left/right/up/down` | `FocusTileDirection` |
| `tile_next`, `tile_prev` | `FocusTabDelta(±1)` |
| `tile_move_left/right/up/down` | `MoveTileDirection` |
| `tile_rename <name>` | `RenameTile` |
| `tile_columns names\|values\|both\|none` | `SetTileColumns` |
| `show_logs`, `show_marker_window`, `show_memory_viewer <array>`, `show_annotation_list` | `OpenTile { … }` (existing names kept) |
| `viewport_add` | alias of `tile_split_right` (linked) |
| `viewport_remove` | `CloseTile(Focused)` if the focused tile is a linked waveform tile |
| `viewport_set_active <n>` | `FocusTile` of the n-th waveform tile in `tile_order()` |

`get_parser` (rebuilt on every keystroke) derives its suggestion lists
(`displayed_items`, `variables_in_active_scope`, markers) from
`target_waveform()`'s item list instead of `waves`. `goto_time`, `zoom_to`,
`zoom_in` etc. send `TileTarget::Focused`.

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
(`J/K/H/L`, digits, arrows) send `TileTarget::Focused`.

Since `TileTarget::Focused` is resolved at `update` time, nothing in the key
handling needs to know about tiles.

### 5.5 Mouse

* Click, right-click or drag start inside a tile → `FocusTile(id)` (detected in
  `render.rs`, not per kind). Hover does **not** move focus.
* Wheel/pinch over a canvas → `CanvasScroll`/`CanvasZoom` with `TileTarget::Id`
  of the hovered tile (as `viewport_idx` today). A tile can be zoomed without
  focusing it.
* Mouse gestures: `gesture_start_location`/`gesture_start_time` on
  `SystemState` gain `gesture_tile: Option<TileId>`; a release over another tile
  cancels the gesture.
* Tab drag-and-drop, split resizing, tab reordering: `egui_tiles`.
* Drag from the hierarchy: the drop target is the tile whose `layout.rect(id)`
  contains the pointer, if it is a waveform tile; else `target_waveform()`.
  `AddDraggedVariables` gains `tile: TileTarget`. The current
  `pointer.x > sidepanel_width` heuristic goes away.

### 5.6 WCP and wasm

WCP keeps working against the **target waveform tile**: `get_item_list`,
`add_variables`, `focus_item`, `set_viewport_to`, `set_viewport_range` operate on
`target_waveform()`. `zoom_to_fit { viewport_idx }` keeps the field, interpreted
as the index into `tile_order()` filtered to waveform tiles; omitted → target.
Later protocol work may add an optional `tile` field; nothing in this design
blocks it. `DisplayedItemRef` in the protocol remains a per-list ref; clients
that manage several lists are out of scope.

`wasm_api::get_state`/`inject_message` are unchanged. `draw_text_arrow` targets
the target waveform's list.

---

## 6. Rendering and egui integration

### 6.1 Frame

`SystemState::draw` keeps its panel order (menu, toolbar, statusbar, overview,
hierarchy sidebar, command prompt, floating dialogs). The former
`variable list` / `variable values` / `Transaction Details` / `Annotation list`
/ `view port N` / `CentralPanel` block is replaced by:

```rust
CentralPanel::default().show(ui, |ui| self.draw_layout(ui, &mut msgs));

fn draw_layout(&mut self, ui: &mut Ui, msgs: &mut Vec<Message>) {
    // Take the runtime tree so that `self` can be borrowed immutably by the panes.
    let mut tree = std::mem::take(&mut self.user.layout.tree);
    let titles = self.tile_titles();               // computed before the pass
    {
        let mut behavior = SurferBehavior { app: &*self, msgs, titles: &titles };
        tree.ui(&mut behavior, ui);
    }
    self.user.layout.tree = tree;
    self.user.layout.remember_rects();             // for hit tests next frame
}
```

Only the tree skeleton is taken; `tiles`, `item_lists` and `waves` stay in
place and readable (the PoC took the content maps too, so code reached from
`pane_ui` saw empty maps). `Layout` methods that touch `tree` must not be
called from inside the pass; the render code reads `focused`, `focus_history`
and `titles` only.

The waveform drawing helpers (`draw_items`, `draw_item_list`, `draw_var_values`,
`generate_draw_commands`) become `&self`; they already keep their caches in
`RefCell`s (`draw_data`, `last_canvas_rect`, `timing`, `flattened_rows_cache`)
and the last `&mut` user, `ensure_drawing_infos_cached`, moves into
`ItemList::layout_cache: RefCell<_>`.

`egui_tiles` mutates the tree directly during `tree.ui` for drag/drop,
resizing and tab clicks. That is the one accepted exception to "mutation only
in `update`": layout geometry is UI state, the same way panel widths are today.
Structural edits initiated by Surfer (`AddTile`, `CloseTile`, `SplitTile`,
`MoveTile`) go through messages so they are undoable and scriptable. Tab
selection by click is reported via `Behavior::on_edit(EditAction::TabSelected)`
and mirrored into `focused` by pushing `FocusTile`.

### 6.2 `Behavior` implementation (`tiles/render.rs`)

```rust
impl egui_tiles::Behavior<TileId> for SurferBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut Ui, _: egui_tiles::TileId, pane: &mut TileId) -> UiResponse {
        let id = *pane;
        let Some(tile) = self.app.user.tiles.get(&id) else { return UiResponse::None };
        let focused = self.app.user.layout.focused == Some(id);
        if !focused && ui.rect_contains_pointer(ui.max_rect())
            && ui.input(|i| i.pointer.any_pressed()) {
            self.msgs.push(Message::FocusTile(id));
        }
        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));   // see 6.3
        let mut cx = TileCtx { app: self.app, tile_id: id, focused, msgs: self.msgs };
        tile.view().ui(ui, &mut cx);
        if focused { paint_focus_frame(ui, &self.app.user.config.theme); }
        UiResponse::None
    }
    fn tab_title_for_pane(&mut self, pane: &TileId) -> WidgetText { self.titles[pane].clone().into() }
    fn is_tab_closable(&self, _: &Tiles<TileId>, _: egui_tiles::TileId) -> bool { true }
    fn on_tab_close(&mut self, tiles: &mut Tiles<TileId>, id: egui_tiles::TileId) -> bool {
        if let Some(pane) = tiles.get_pane(&id) { self.msgs.push(Message::CloseTile(TileTarget::Id(*pane))); }
        false   // the message does the removal
    }
    fn on_tab_button(&mut self, tiles, id, button: Response) -> Response {
        button.context_menu(|ui| self.tab_context_menu(ui, tiles, id)); button
    }
    fn top_bar_right_ui(&mut self, tiles, ui, tabs_id, _tabs, _scroll) {
        // "+" menu: new tile kinds, split right/down for the active tab
    }
    fn tab_bar_height(&self, style) -> f32 {
        if self.single_tile && self.app.user.config.layout.hide_single_tab_bar() { 0.0 }
        else { self.app.user.config.layout.tab_bar_height }
    }
    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions { all_panes_must_have_tabs: true, prune_empty_tabs: true,
            prune_single_child_tabs: false, prune_empty_containers: true,
            prune_single_child_containers: true, join_nested_linear_containers: true }
    }
    fn on_edit(&mut self, action: EditAction) { /* TabSelected → FocusTile of the new active pane */ }
    fn is_tile_draggable(&self, ..) -> bool { !self.single_tile }
    // colours from SurferTheme: tab_bar_color, tab_bg_color, tab_text_color, resize_stroke
}
```

`all_panes_must_have_tabs: true` means every tile always sits in a tab group,
so there is **one** rendering path whether there is one tile or ten. The only
thing that changes for a lone tile is `tab_bar_height` (config
`layout.hide_single_tab_bar`, default `false`). The PoC's separate
`is_single_waveform` fast path is not reproduced.

Tab context menu (generic part): Split Right, Split Down, Split Copy Right/Down
(waveform), Close, Close Others, Rename…, then `tab_context_menu` of the kind.

### 6.3 egui ids and clipping

* Every egui id inside a tile derives from the pane `Ui` (`ui.id()`), which
  `egui_tiles` salts with its `TileId`. Panels inside the waveform tile become
  `Panel::left(ui.id().with("names"))` and `Panel::left(ui.id().with("values"))`;
  every `ScrollArea` gets `id_salt(cx.id("…"))`. No global string ids inside
  tiles.
* Widget-focus bookkeeping keyed by string (`text_edit_focused`, `time_widgets`)
  uses `format!("tile{}/{}", id.0, name)` keys via `cx.id_str(name)`.
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
* Item context menu: "Show memory viewer" → `AddTile { Memory, Beside(Right) }`.
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
split, or split the group edge). Every operation is a `MoveTile` with a
`Placement` so scripts can do it too.

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

The palette always acts on `TileTarget::Focused`. Its suggestion lists (items,
markers, variables in scope) come from the target waveform. Kind-specific
commands (`memory_goto <index>`, future `table_sort`) are registered by the
kind and are only offered when the focused tile is of that kind: `get_parser`
asks `focused tile → view().commands()` (an optional trait method returning
`Vec<(&'static str, Parser)>`). This keeps `command_parser.rs` free of
per-kind knowledge.

### 7.8 State files

`Ctrl+S` writes the whole workspace. Loading a `.surf.ron` restores layout,
tiles and item lists and reattaches them to the loaded waves. The sibling
state file mechanism (`<wave>.surf.ron` next to the wave) is unchanged and now
restores the full layout.

---

## 8. Serialization

### 8.1 Format

RON, same file, same extension. Example (abridged):

```ron
UserState(
    version: 1,
    layout: Layout(
        root: Some(Split(
            dir: Horizontal,
            shares: [0.65, 0.35],
            children: [
                Tabs(active: 0, children: [Tile(TileId(1)), Tile(TileId(3))]),
                Split(dir: Vertical, shares: [0.5, 0.5], children: [
                    Tabs(active: 0, children: [Tile(TileId(2))]),
                    Tabs(active: 0, children: [Tile(TileId(4))]),
                ]),
            ],
        )),
        focused: Some(TileId(1)),
        focus_history: [TileId(1), TileId(4), TileId(2)],
    ),
    tiles: {
        TileId(1): Waveform(WaveformTile(items: ItemListId(1), viewport: Viewport(...), scroll_offset: 0.0, focused_item: None, ...)),
        TileId(2): Waveform(WaveformTile(items: ItemListId(1), viewport: Viewport(...), show_name_column: false, ...)),
        TileId(3): Waveform(WaveformTile(items: ItemListId(2), ...)),
        TileId(4): Memory(MemoryTile(scope: ScopeRef(...), name: "mem", value_format: "Hexadecimal", ...)),
    },
    item_lists: {
        ItemListId(1): ItemList(items_tree: DisplayedItemTree(...), displayed_items: {...}, ref_counter: 12, annotations: [], ...),
        ItemListId(2): ItemList(...),
    },
    waves: Some(WaveData(source: File("cpu.vcd"), format: Vcd, cursor: Some(1200), markers: {0: 800}, ...)),
    // …existing preference fields unchanged…
)
```

Rules:

* `#[serde(default)]` on every struct and on `UserState` (already the case),
  so a missing field never fails a load.
* `tiles` and `item_lists` are `BTreeMap`s: stable ordering in the file, small
  diffs.
* Tile kinds serialize as externally tagged enum variants (`Memory(...)`).
  Kind state structs own their format; the layout core does not care.
* `Layout` serializes as `LayoutNode`, never as `egui_tiles` internals.
* Runtime fields are `#[serde(skip)]`.
* `version: u32` is added to `UserState` (`default = 0` for files that predate
  it). Migrations run in `load_state` in order (`0 → 1`: §8.3). The number is
  bumped only for changes `#[serde(default)]` cannot express.

### 8.2 Evolution

* Adding a field to a tile kind: give it a default. Nothing else.
* Adding a kind: new variant. Older Surfer versions reading such a file would
  fail on the unknown variant; to prevent that, `tiles` is deserialized entry by
  entry through a helper that first reads a `ron::Value` and tries
  `TileKind::deserialize`; on failure the entry becomes
  `TileKind::Unknown(UnknownTile { kind: String, raw: String })`, rendered as a
  placeholder ("This tile was saved by a newer Surfer") and written back out
  verbatim on save, so a round trip through an old version does not lose it.
* Removing a kind: keep the variant name in a `LEGACY_KINDS` list that maps to
  `Unknown`, or provide a migration.
* Changing the layout shape: `LayoutNode` is the contract; `egui_tiles` can be
  swapped for another engine without touching files.

### 8.3 Legacy migration (`version 0` files)

A `version 0` file has `waves.items_tree`, `waves.displayed_items`,
`waves.viewports`, `waves.cursor`, … and no `layout`. `WaveData` keeps the moved
fields as `#[serde(default, skip_serializing)] legacy_*` mirrors for one
release cycle. Migration in `load_state`:

1. If `layout.root.is_none()` and `waves.legacy_items_tree` is non-empty:
   build `ItemList(1)` from the legacy item fields, annotations and graphics.
2. For each legacy viewport `i`: `WaveformTile { items: 1, viewport: viewports[i],
   scroll_offset, focused_item, focused_transaction }` as tile `i+1`, with
   `show_name_column = show_value_column = (i == 0)` to reproduce the old
   look. Layout: `Split { Horizontal, equal shares, [Tabs(1), Tabs(2), …] }`.
   `focused = last_active_viewport_idx + 1`.
3. `show_cursor_window`, `show_logs`, `show_annotation_list` → `OpenTile` of
   the corresponding kind at `Edge(Right)`/`Edge(Down)`, unfocused. (These
   flags were never meant to be persistent UI; migrating them is cheap and
   surprises nobody.)
4. Clear the legacy fields, set `version = 1`.

`.sucl` command files need no migration: old command names remain aliases.

`UserState::previous_waves` (used by the startup path to carry a state file's
presentation until the wave finishes loading) is replaced by
`pending_state: Option<Box<UserState>>` holding the whole loaded state; on
`WavesLoaded` the reattachment in §11.3 runs against it.

---

## 9. Undo / redo

`CanvasState` grows to a workspace snapshot:

```rust
struct CanvasState {
    message: String,
    layout: LayoutNode-or-Layout clone,
    tiles: BTreeMap<TileId, TileKind>,
    item_lists: BTreeMap<ItemListId, ItemList>,
    markers: HashMap<u8, BigInt>,
}
```

`TileKind: Clone` and `ItemList: Clone` drop runtime caches in their `Clone`
impls, as `AnalogVarState::clone` does today. Snapshots are taken where they are
taken today (item edits, marker edits, annotation edits) plus `AddTile`,
`CloseTile`, `SplitTile`, `MoveTile`. Zoom, scroll, focus and split resizing
stay outside undo, as now. `undo_stack_size` applies unchanged; memory cost is
dominated by item lists, which were already cloned per snapshot.

---

## 10. Adding a new tile kind

Checklist — everything is in the kind's module except items 4 and 5:

1. `tile_kinds/<kind>.rs`: state struct (`Serialize, Deserialize, Clone,
   Default`), its `<Kind>Message` enum, `impl TileView`, `fn update(&mut self,
   <Kind>Message, &mut TileUpdateCtx)`, optional `commands()` for the palette,
   optional runtime cache in a `#[serde(skip)] RefCell`.
2. Tests: a snapshot test that builds the layout with `SetLayout`/`AddTile`
   and a serde round trip of the state struct.
3. Docs: a section in `docs/` and the kind in the tile list.
4. `tiles/kind.rs`: one variant in `TileKind`, one in `TileMessage`, one arm in
   `view`/`view_mut`/`update`, one entry in `CREATABLE`.
5. `menus.rs`: nothing, if the kind is creatable from the generic **New Tile**
   menu. Context-menu entry points (e.g. "Show memory viewer" on an item) are
   added where the entry point lives.

Nothing in `layout.rs`, `render.rs`, `serde.rs`, `state.rs`, `lib.rs::update`
or `command_parser.rs` changes.

### 10.1 Worked example: the memory viewer

Today: `MemoryViewerState` on `SystemState` (`memory_viewer.rs:31`), one
instance, opened by `Message::OpenMemoryViewer { scope, name }`, drawn as an
`egui::Window("Memory Viewer")`, cache in `SystemState::memory_viewer_cache`,
settings changed by mutating the struct inside the window closure.

After:

```rust
// tile_kinds/memory.rs
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct MemoryTile {
    pub scope: Option<ScopeRef>,
    pub name: Option<String>,
    pub index_format: MemoryViewerFormat,
    pub value_format: String,
    pub value_column_count: usize,
    pub color_values: bool,
    pub search: ValueQuery, pub highlight: ValueQuery, pub filter: ValueQuery,   // value + match mode + case
    pub filter_mode: ChangeModes, pub highlight_mode: ChangeModes,
    #[serde(skip)] scroll_to_row: Cell<Option<usize>>,
    #[serde(skip)] cache: RefCell<Option<MemoryViewerCache>>,     // keyed by (cursor, generation, array)
}

#[derive(Debug, Deserialize)]
pub enum MemoryMessage {
    SetArray { scope: ScopeRef, name: String },
    SetIndexFormat(MemoryViewerFormat), SetValueFormat(String),
    SetColumnCount(usize), SetColorValues(bool),
    SetSearch(ValueQuery), SetHighlight(ValueQuery), SetFilter(ValueQuery),
    JumpToIndex(usize), SelectValue(Option<usize>),
}

impl TileView for MemoryTile {
    fn kind_name(&self) -> &'static str { "memory" }
    fn title(&self, _: &TileCtx) -> String {
        match &self.name { Some(n) => format!("Memory: {n}"), None => "Memory".into() }
    }
    fn ui(&self, ui: &mut Ui, cx: &mut TileCtx) {
        let Some(waves) = cx.waves() else { ui.label("No file loaded"); return };
        let Some(cursor) = &waves.cursor else { ui.label("Place the cursor to inspect values."); return };
        // toolbar row: array picker, formats, filters — every change → cx.send_self(MemoryMessage::…)
        // table: egui_extras::TableBuilder with id_salt(cx.id("table")); rows from self.cache (rebuilt if key differs)
    }
    fn split_clone(&self) -> Option<TileKind> { Some(TileKind::Memory(self.clone())) }
    fn on_waves_changed(&mut self, _: WavesChange, cx: &mut TileUpdateCtx) -> bool {
        self.cache.borrow_mut().take();
        self.name.as_ref().map_or(true, |n| cx.array_exists(self.scope.as_ref(), n))
    }
}

impl MemoryTile {
    pub fn update(&mut self, m: MemoryMessage, _: &mut TileUpdateCtx) {
        match m { MemoryMessage::SetArray { scope, name } => { self.scope = Some(scope); self.name = Some(name); self.cache.borrow_mut().take(); }
                  /* … field assignments … */ }
    }
}
```

Entry point: the item context menu's "Show memory viewer" sends
`AddTile { kind: TileKind::Memory(MemoryTile { scope, name, ..Default::default() }), placement: Beside(focused, Right), focus: true }`.
Removed: `MemoryViewerState::open`, `SystemState::memory_viewer`,
`memory_viewer_cache`, `Message::OpenMemoryViewer`, the window in `view.rs:247`.
The rendering code moves with minimal edits: direct field writes become
`cx.send_self(..)`.

### 10.2 Sketch: a signal change table

A future `SignalTableTile { variables: Vec<VariableRef>, sort: Option<(usize, bool)>, filter: String, columns: Vec<ColumnKey> }`
whose `ui` renders an `egui_extras` table of value changes. Rows are produced
by a model built off-thread from `SignalAccessor` snapshots (formatted strings,
`Send + Sync`), cached in a `#[serde(skip)] RefCell<Option<Arc<TableCache>>>`
keyed by `(variables, translator, cache_generation)`; build requests go through
`Message::ToTile(id, SignalTableMessage::CacheBuilt(Arc<..>))` from a
`tokio`/`rayon` task, following the `BuildAnalogCache`/`AnalogCacheBuilt`
pattern. Row activation sends `Message::CursorSet(time)` — tables push time to
the shared cursor, and follow it by highlighting the row at `waves.cursor`.
`is_ready()` returns false while a build is in flight so snapshot tests can
wait. This is the PoC's `TableModel`/cache protocol, reduced to what the tile
contract needs; the table subsystem itself is separate work.

---

## 11. Migration path

Ordered so that each step compiles, passes tests and could ship.

### 11.1 Step 1 — `ItemList` extraction (no UI change)

Move the item-anchored fields out of `WaveData` into `ItemList`; `WaveData`
holds `item_list: ItemList` temporarily. Change method receivers from
`&WaveData` to `&ItemList` where they only touch items. Update
`update_with_items`, undo, tests. Purely mechanical; snapshots unchanged.

### 11.2 Step 2 — Layout, `TileId`, waveform tile, `egui_tiles`

* Add `egui_tiles = { version = "0.17", default-features = false }` (targets
  egui 0.36; its `serde` feature is not needed because `Layout` owns the format).
* Add `tiles/` and `tile_kinds/waveform.rs`. `UserState` gets `layout`,
  `tiles`, `item_lists`, `version`. `WaveData` loses `viewports`,
  `last_active_viewport_idx`, `scroll_offset`, `focused_item`,
  `focused_transaction`, `item_list`.
* Move the name/value/canvas panels from `view.rs` into `WaveformTile::ui`;
  make `draw_items` and friends `&self`; per-tile draw cache.
* Replace `viewport_idx` in messages with `TileTarget`; delete the viewport
  messages; add the layout messages; `TileTarget::Focused` resolution.
* Serialization, legacy migration, `SetLayout`.
* Snapshot tests: the three viewport tests become linked-split tests; new tests
  for split/tabs/close/focus and legacy-file loading. Most existing snapshots
  change only by the tab bar (or not at all with `hide_single_tab_bar`).

### 11.3 Reattachment on load (part of step 2)

`SystemState::on_waves_loaded(new_waves, load_options)`:

* `LoadOptions::Clear` (a different file): reset to the default layout — one
  empty waveform tile, empty item list. Non-waveform tiles are dropped (their
  targets belong to the old file).
* `KeepAvailable`/`KeepAll` (reload, switch_file): keep layout and tiles; for
  every `ItemList` run `update_with_items(keep_unavailable)`; clip every
  waveform tile's `Viewport` to the new time range; call
  `on_waves_changed(Reloaded)` on every tile and close those returning false.
* A pending state file (`pending_state`): apply its layout/tiles/lists, then the
  same per-list reattachment.

### 11.4 Step 3 — Widget migration

Memory viewer, markers window, logs window, frame buffer window, annotation
list panel, transaction details panel → tile kinds, one commit each. Each
commit deletes a `show_*` flag or a `SystemState` field and its window.

### 11.5 Step 4 — Polish

Keyboard navigation and move, drag from hierarchy onto a specific tile,
overview click-to-focus, per-kind palette commands, WCP `viewport_idx`
semantics, docs (`docs/tiles.md`, updates to `docs/commands`).

### 11.6 Step 5 — New kinds

Signal tables, pipeline/event views, on the contract of §10, independently.

---

## 12. Testing

* **Unit**: `LayoutNode` ↔ tree round trip; `Placement` insertions;
  `neighbor()` on a fixed layout; `CloseTile` focus fallback; legacy `version 0`
  fixture files under `libsurfer/src/tests/state_files/` load into the expected
  layout; unknown-kind round trip.
* **Snapshot** (`snapshot_ui_with_file_and_msgs!`): layouts built with
  `Message::SetLayout` + `AddTile` so tests are deterministic; linked vs copied
  splits; focus frame; hidden columns; every migrated kind in a tile;
  `tab_bar` on/off. `wait_for_waves_fully_loaded` additionally waits until every
  tile's `is_ready()` is true.
* **Interaction** (real input, like `theme_menu_radio_button`): click focuses a
  tile; tab click; tab close button. Drag-and-drop docking is `egui_tiles`'
  responsibility and is not snapshot-tested here.
* **WCP** tests unchanged in intent; `zoom_to_fit { viewport_idx: 1 }` against a
  two-tile layout.

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

## 14. Open questions and trade-offs

1. **Linked item lists vs. copies only.** Linked lists (`ItemListId`
   indirection) preserve today's multi-viewport workflow and cost one map and
   one id. Dropping them would simplify undo and WCP slightly but regress a
   shipped feature. Recommendation: keep linked lists.
2. **Cursor per tile?** Some tools (GTKWave, Verdi) have one cursor; some users
   ask for independent cursors in compare views. This design keeps one shared
   cursor; a per-tile "secondary cursor" could be added later as tile state
   without a format change. Recommendation: shared only, revisit with user
   feedback.
3. **Zoom sync between waveform tiles.** Useful for side-by-side comparison of
   different signal sets over the same window. Proposed later addition:
   `sync_group: Option<u8>` on `WaveformTile`; after messages are applied, tiles
   in the same group adopt the window of the tile last changed (the PoC's
   `viewport_sync` arbitration, simplified because all participants share one
   time base). Not in the first version.
4. **Tab bar on a single tile.** Always showing it is uniform and discoverable
   (VSCode); hiding it keeps the current look. Config option, default show.
   Decide after trying it.
5. **`Ctrl+W` in the browser.** Cannot be intercepted reliably. Alternatives:
   no default binding in wasm, or `Ctrl+Shift+W`. Recommendation: bind
   `Command+W`, document the limitation, rely on the tab ✕.
6. **Where does the hierarchy sidebar belong?** Left out of the tree (VSCode
   sidebar model) so that "add variable" has an unambiguous target and the
   tree never becomes empty of a place to add things. Making it a tile is
   possible later since it is state-free apart from the filter; nothing in
   the format prevents it.
7. **Undo of tile operations.** Included (cheap, consistent with item undo).
   Layout resizing and tab reordering are not undoable, like panel widths.
   Alternative: exclude tiles from undo entirely and rely on "Reopen closed
   tile". Recommendation: include.
8. **WCP tile addressing.** Keep `viewport_idx` as "n-th waveform tile" for
   compatibility now; add an optional `tile: u64` field when a client needs
   it. Protocol change requires coordination with `surfer-wcp` consumers.
9. **`LoadOptions::Clear` resets the layout.** Simple and predictable, but a
   user who arranged tiles and opens a different design loses the arrangement.
   Alternative: keep the layout and empty the lists. Recommendation: reset;
   sibling state files restore per-design layouts anyway.
10. **Per-kind palette commands via `commands()`.** Keeps `command_parser.rs`
    kind-agnostic but means a kind's commands are only reachable when it is
    focused. Global variants (e.g. `memory_goto` targeting the last-focused
    memory tile) can be added per kind if needed.
