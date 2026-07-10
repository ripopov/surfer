# Surfer Konata View — UI/UX Specification

**Status:** Design proposal (target user experience; no implementation details)
**Audience:** Surfer users, UX reviewers, and implementers validating behavior
**Input format:** FTR transaction traces following the pipeline-trace convention
(e.g. `examples/kanata-sample-2.ftr`)

This document specifies a new *Konata view* for Surfer: an instruction-pipeline
visualization pane, inspired by (and targeting user-visible feature parity with)
[Konata](https://github.com/shioyadan/Konata), the pipeline viewer for
Onikiri2-Kanata and gem5-O3PipeView logs. The Konata view becomes the third way
to look at transaction data in Surfer, alongside the existing **waveform view**
and **table view**.

---

## 1. Why a Konata view?

### 1.1 The problem

A CPU pipeline trace answers questions like:

* *Why does this loop take 12 cycles per iteration instead of 4?*
* *Where did the flush after this mispredicted branch start, and how many
  instructions did it kill?*
* *Which dependency chain serializes these instructions?*
* *Is the frontend or the backend the bottleneck in this region?*

Surfer can already load such traces (one transaction per executed instruction,
one event per pipeline stage), but neither existing view answers these
questions well:

* The **waveform view** plots transactions against *time* on shared stream
  rows. Thousands of overlapping instructions collapse into a solid smear —
  the current rendering of `kanata-sample-2.ftr` is a wall of green boxes
  (see `snapshots/kanata_pipeline_trace_renders.png`). Time-on-X is the right
  axis for signals and bus transactions, but pipeline analysis needs one row
  *per instruction*.
* The **table view** shows instructions and stage events as rows with
  sortable/filterable columns. Excellent for querying ("show all flushed
  stores"), but it has no notion of *shape*: you cannot see stalls, bubbles,
  flush wavefronts, or dependency diagonals in a grid of numbers.

### 1.2 The Konata idea

A pipeline diagram uses a different projection:

* **X axis = clock cycles**
* **Y axis = instructions, one row each, in fetch order**
* Each row shows that instruction's pipeline stages as colored boxes.

In this projection, microarchitectural behavior becomes *visible texture*:

```text
            cycle →
  insn ↓   0    1    2    3    4    5    6    7    8    9
  i0      [F ][Dc][Rn][Is][X ][Cm]
  i1           [F ][Dc][Rn][Is][X ][Cm]
  i2                [F ][Dc][Rn][.........][Is][X ][Cm]      ← stall: long box
  i3                     [F ][Dc][Rn][....][Is][X ][Cm]
  i4                          [F ][Dc]▓▓▓▓▓▓▓▓▓▓             ← flushed: dimmed
  i5                               [F ][Dc]▓▓▓▓▓▓▓
  i6                                         [F ][Dc][Rn]…   ← refetch after flush
```

* A smoothly descending staircase = healthy pipelining.
* A vertical cliff = stall; its width is the penalty in cycles.
* A dark triangle = flush; its height is the number of squashed instructions.
* Arrows between rows = data dependencies; long arrows = serialization.

This is the view Konata provides for Onikiri and gem5 logs. Surfer's Konata
view brings the same projection to **FTR traces from any simulator** — RTL,
SystemC/TLM, performance models — through the neutral FTR pipeline-trace
convention, while integrating with everything Surfer already does: tiles,
tables, markers, themes, remote traces, and scripting.

### 1.3 What Surfer adds over standalone Konata

* **One tool, three projections.** The same trace can be open in the waveform
  view (time-oriented), the Konata view (pipeline-oriented), and table
  views (query-oriented) simultaneously, with cross-navigation between them.
* **A structured input format.** FTR carries typed attributes and explicit
  relations. Dependencies, retirement status, and thread IDs can be
  first-class data; any heuristic fallback is explicit and labeled.
* **Tiling instead of app windows.** Konata views are tiles: dock them beside
  waveforms, stack two traces for comparison, or maximize one full-screen.
* **Integrated Surfer workflows:** fuzzy command prompt, saved sessions
  (`.surf.ron` state files), light/dark themes, remote traces via Surver,
  WCP scripting, reproducible snapshot testing.

### 1.4 Design principles

1. **The trace remains the source of truth.** The view never moves, stretches,
   or silently drops a recorded stage to make the diagram look cleaner.
2. **Identity survives projection changes.** Filtering, sorting, hiding flushed
   ops, and switching between views never changes an instruction's ID, SID,
   RID, or FTR transaction identity.
3. **Useful before perfect.** A partially loaded or partially conforming trace
   remains navigable, with missing or malformed data called out in place.
4. **Color is supplemental.** Text, geometry, outlines, and state badges carry
   the meaning needed to use the view without relying on hue alone.
5. **Large traces are normal.** Progressive results, cancellable searches, and
   zoom-dependent detail are baseline behavior rather than optional polish.

Unless a sentence explicitly says "may" or "optional," this document describes
the target behavior required for the feature to be considered complete. The
parity matrix in §15 is a coverage checklist, not a claim about the current
implementation.

---

## 2. The data model, as the user sees it

The Konata view consumes FTR generators that follow the **pipeline-trace
convention** (this is what the `konata2ftr` converter emits, and what any
simulator can emit directly with an FTR writer):

| Concept in the view | FTR representation |
|---|---|
| Instruction (a row) | Transaction in a parent generator, e.g. `pipeline.instruction`; spans fetch→retire |
| Pipeline stage (a box) | Event transaction in the matching `instruction.events` generator, linked by `parent_of` with the instruction as source and event as sink |
| Stage name (`F`, `Dc`, `X`…) | The event's `name` attribute |
| Lane (parallel sub-rows) | The event's `lane` attribute (optional; defaults to a single lane) |
| Disassembly / left-pane label | Instruction attribute `label` |
| Mouse-over details | Instruction attribute `detail` (multi-line) |
| Per-stage annotations | Repeated `label` attributes, plus any other attributes, on the stage event |
| Simulator serial number | Instruction attribute `insn_id_in_sim` |
| Thread ID | Instruction attribute `thread_id` |
| Retirement order | Instruction attribute `retire_id` |
| Flushed (squashed) op | Instruction attribute `flushed` (boolean) |
| Dependency arrow | Relation between two instruction transactions (e.g. `wakeup`) |

Terminology used throughout this document, matching Konata:

* **ID** — stable, zero-based position of the instruction in fetch order.
* **SID** (serial ID) — `insn_id_in_sim`, the simulator's own numbering.
* **RID** (retire ID) — `retire_id`, position in retirement order. Flushed
  ops have no meaningful RID.
* **Flushed op** — an instruction that was fetched but squashed, never
  retired.

The pipeline convention defines the parent generator's recorded transaction
begin order as fetch order. The view preserves that order even when timestamps
tie or move backward; a backward timestamp receives a data-quality warning
rather than causing IDs to change. This keeps IDs stable during progressive
loading. Filtering or hiding rows does not renumber them. RID ordering is per
thread; commands and comparison controls therefore address it as
`(thread_id, retire_id)` whenever more than one thread is present.

The pipeline convention also supplies a **clock mapping** for each instruction
generator: a positive clock period and an optional phase/origin. A cycle is the
half-open interval `[origin + n×period, origin + (n+1)×period)`. If that mapping
is absent, the view opens in trace-time mode and offers **Set pipeline clock…**;
cycle labels, cycle jumps, and IPC stay unavailable until the user chooses a
valid period and origin. An FTR timescale alone converts ticks to seconds but
does not define a clock period.

Any generator pair that follows the convention gets the Konata view — the
trace does not have to come from a CPU. Anything with "items that flow through
named phases" fits: GPU wavefronts, NoC packets with per-hop events,
accelerator command queues, and network requests. CPU-specific fields are
optional: missing SID, RID, thread ID, disassembly, or flushed state use the
fallbacks defined below rather than disabling the view.

---

## 3. Opening a Konata view

There are four equivalent entry points; all create a **Konata tile** in the
central tile area:

1. **Sidebar context menu.** Right-click a generator (or its stream) in the
   *Streams* sidebar → **"Open in Konata view"**. The item is enabled whenever
   the generator has a matching `.events` companion in the same stream.
2. **Menus.** *View → New Konata view…* lists all conforming generators in
   the loaded trace.
3. **Command prompt.** `konata_view_new <generator>` with fuzzy completion of
   generator names, consistent with existing `table_new`-style commands.
4. **Automatic suggestion.** When a loaded FTR file contains at least one
   matching generator pair and the user adds it to the waveform view, the
   status bar offers a one-click hint: *"Pipeline trace detected — open in
   Konata view?"* (dismissible, never modal).

The generator pair is enough to make the entry point available because a
progressive load may not have parsed any relations yet. After opening, the view
validates event parentage and stage identity as data arrives. A pair with
no usable stage events shows the empty state *"No pipeline stages found"* with
links to the raw event table and data-quality summary; it never presents an
apparently valid blank canvas.

Like all Surfer tiles, a Konata tile can be split left/right/top/bottom of
other tiles, moved into a tab group, resized, and closed. The tile title shows
the generator name and file, e.g. `Konata — pipeline.instruction
(kanata-sample-2.ftr)`. Several Konata tiles can be open at once, over the
same or different traces.

The complete view state (open Konata tiles, scroll/zoom positions, options,
color scheme, bookmarks) is saved in Surfer state files and restored with the
session.

---

## 4. Anatomy of the view

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Konata — pipeline.instruction (kanata-sample-2.ftr)                 [tile] │
├───────────────────────────────┬─┬──────────────────────────────────────────┤
│ ① Label pane                  │②│ ③ Cycle ruler   2880........2900........ │
│                               │ ├──────────────────────────────────────────┤
│ 1350: s5720 (t0:r1146)        │s│  ④ Pipeline canvas                       │
│   0000220c: addi a4, zero,0x7 │p│   [Np][F][Pd][Dc][Rn][Ds][Sc][Is][Rr][X] │
│ 1351: s5724 (t0:r1147)        │l│      [Np][F][Pd][Dc][Rn][Ds][Sc][Is][Rr] │
│   00002210: addi a3, zero,0x8 │i│         [Np][F][Pd][Dc][Rn][Ds][Sc][1][2]│
│ 1352: s5728 (t0:r1148)        │t│            [Np][F][Pd][Dc][Rn]▓▓▓▓▓▓▓▓▓  │
│   00002214: addi a2, a2, 0x1  │t│               ╰──────────➤ (dep arrow)   │
│ ...                           │e│  ...                                     │
│                               │r│                                          │
├───────────────────────────────┴─┴──────────────────────────────────────────┤
│ ⑤ Status strip: [cycle 2887, ID 1350]  X[1]  zoom 1:1  4041 ops (374 fl)  │
└────────────────────────────────────────────────────────────────────────────┘
```

### ① Label pane (left)

One entry per instruction, aligned with the canvas rows:

```
<ID>: s<SID> (t<TID>: r<RID>): <label>
1351: s5724 (t0: r1147): 00002210: addi a3, zero, 0x8
```

* The label is the instruction's `label` attribute — typically PC and
  disassembly. A missing label displays `tx#<FTR transaction ID>`.
* Missing SID, TID, or RID fields are omitted rather than rendered as invented
  zeroes. A flushed op or an op without a retire ID shows `r—`.
* **Click a row** to focus it and scroll the canvas horizontally so that the
  instruction's fetch cycle is at the left edge ("take me to where this op
  executes"). Clicking its stage on the canvas focuses without realigning.
* **Hover** shows a tooltip with the full identity: label, all `detail`
  lines, SID, thread ID, RID, and a `flushed` note where applicable.
* The pane hides its text when rows become shorter than the readable
  threshold (deep zoom-out) and reappears on zoom-in.
* Right-click a row: *Copy label*, *Copy row as text*, *Focus transaction*,
  *Show in event table*, *Show in waveform view* (§10).

### ② Splitter

A draggable vertical splitter between label pane and canvas; position is
per-tile and persisted. Double-click resets to the default split. When two
Konata tiles are scroll-synchronized (§9), their splitters move together so
the canvases stay comparable.

### ③ Cycle ruler

A horizontal ruler across the top of the canvas showing cycle numbers, with
tick density adapting to zoom. When a pipeline clock is available, the ruler
can display **cycles** (default) or **wall-clock time** using Surfer's existing
time-unit setting (`ns`, `µs`, …). Without a clock mapping, it displays
wall-clock time and offers **Set pipeline clock…**. The ruler context menu
switches units. Surfer cursors and markers that fall inside the visible range
appear as labeled pins (§10); coincident pins stack instead of occluding one
another.

### ④ Pipeline canvas

The main drawing area; behavior detailed in §5–§8.

### ⑤ Status strip

A one-line readout at the bottom of the tile (not the global status bar):
current mouse position as `[cycle, ID]`, the stage under the cursor with its
duration (`X[1]`), current zoom, and trace totals (`4041 ops, 374 flushed`).
During loading, totals are prefixed with `loaded` and remain visibly
provisional. In trace-time mode, the position uses the selected time unit
instead of `cycle`.

---

## 5. The pipeline canvas

### 5.1 Rows and stage boxes

* Each visible instruction occupies one horizontal row; row height and column
  width scale with zoom (nominal 24 px row, 32 px per cycle at zoom 1:1).
* Each non-zero-duration stage event draws over its recorded half-open time
  range `[start, end)`. Its left and right edges are placed at their exact
  timestamps, even when they fall between cycle boundaries:
  * stage **name** centered in the first cycle's cell (`F`, `Dc`, `X`…);
  * multi-cycle stages additionally number their trailing cells `1 2 3 …` so
    stall lengths can be read directly off the picture (a 6-cycle `Sc` reads
    `Sc 1 2 3 4 5`);
  * a subtle vertical gradient and a thin border delimit adjacent stages.
  In trace-time mode the geometry remains exact, but cycle-cell numbering is
  hidden until a pipeline clock is configured.
* A zero-duration stage draws as a narrow diamond at its timestamp. It remains
  hoverable and focusable, so point events from the standard FTR event
  convention are not lost.
* Overlapping stages in the same lane remain distinct through vertical inset
  and draw order. The tooltip lists all hits; clicking repeatedly cycles
  through them, and the context menu lists them by name and timestamp.
* Every other row carries a faint background stripe for horizontal eye
  tracking.
* The regions above the first instruction and below the last are shaded
  darker, so "off the edge of the trace" is unambiguous.
* **Flushed instructions** render with a dark translucent overlay across all
  their stages — flush triangles stand out at any zoom.
* Events outside their parent instruction's time range remain at their
  recorded timestamps and gain the same warning treatment used by Surfer's
  waveform event view. They are never clipped to the parent.
* While a large file is still loading, the canvas already shows everything
  parsed so far; rows appear progressively and the tile shows a progress bar
  (Surfer's standard async-loading behavior).

### 5.2 Level-of-detail (zoom adaptive rendering)

The canvas degrades gracefully over roughly 24 half-step zoom levels
(wheel/keys use the configured step; pinch zoom is continuous). A level of
detail is selected from the smaller of row height and the on-screen width of a
one-cycle stage, so narrow columns cannot retain unreadable text merely because
rows are tall:

| Effective detail size | What is drawn |
|---|---|
| ≥ ~10 px | Full detail: colored stages, borders, names, cycle numbers |
| ~4–10 px | Colored stages + borders, no text |
| ~1–4 px | Colored stage strips, no borders; dependency arrows hidden |
| < ~1 px | Density envelope over fetch→retire extents; individual rows are sampled only for hit-testing after zoom-in |

At maximum zoom-out the entire loaded trace can be fit as a density skyline:
dense diagonal texture suggests throughput; horizontal gashes identify long
stalls; dark wedges identify flush storms. Aggregated pixels expose their
instruction/time range and count in the tooltip, and clicking one zooms into
that range. All four thresholds are user-configurable (§12).

### 5.3 Lanes

Traces may attach a `lane` to each stage event (e.g. Onikiri uses lanes for
overlapping activities like a re-scheduled op, or scalar vs. memory
pipelines).

* **Merged (default):** all lanes draw inside the instruction's single row,
  inset and layered in lane order. Overlap is never communicated by opacity
  alone.
* **Split lanes** (toggle `n`): each lane becomes its own sub-row. Two
  sub-modes:
  * *Natural height* — each op grows to `lanes × row height`;
  * *Fixed op height* — each op keeps one row height, lanes subdivide it.
* Lane identity contributes a stable hue offset, so lane 1 stages are
  distinguishable from lane 0 at a glance.

---

## 6. Navigation

Design goal: identical muscle memory to Konata, harmonized with Surfer's
existing bindings. All motion (scroll, zoom, jumps) is smoothly animated
(~80–100 ms ease-out); animations are instant when the reduced-motion setting
is on.

### 6.1 The diagonal follow

The defining Konata navigation trick, preserved exactly: because pipelines
advance in both axes at once, **vertical scrolling automatically compensates
horizontally** to follow the pipeline diagonal. Scrolling down the wheel keeps
the "current" instructions on screen instead of letting them run off to the
right. Hold **Shift** to scroll strictly vertically without the compensation.

The compensation preserves the fetch position of the instruction nearest the
viewport center. It is based on actual adjacent fetch timestamps rather than an
assumed one-instruction-per-cycle slope, so bubbles, simultaneous fetches, and
variable-width frontends do not make the view drift. At the first/last row or
when all visible rows share a timestamp, compensation becomes zero.

### 6.2 Mouse and touch

| Input | Action |
|---|---|
| Wheel | Scroll rows (diagonal-following) |
| Shift + wheel | Scroll rows, no horizontal compensation |
| Horizontal wheel / touchpad | Scroll cycles |
| Ctrl + wheel | Zoom the time axis centered on the pointer |
| Alt + wheel | Zoom both axes centered on the pointer |
| Left-drag | Pan freely in both axes |
| Double-click | Zoom in at the pointer; Shift+double-click zooms out |
| Pinch | Continuous zoom |
| Right-click | Context menu (§6.5) |

### 6.3 Keyboard

Active while the Konata tile has focus; follows Konata with Surfer-consistent
additions:

| Key | Action |
|---|---|
| `↑` / `↓` | Scroll rows (diagonal-following; Shift disables compensation) |
| `←` / `→` | Scroll cycles |
| `PageUp` / `PageDown` | Scroll one viewport, retaining one context row |
| `Ctrl+↑` / `Ctrl+↓`, `+` / `-` | Zoom in / out at view center |
| `Home` / `End` | Jump to first / last instruction |
| `n` | Toggle split lanes |
| `f` or `Ctrl+F` | Find (§7.3) |
| `F3` / `Shift+F3` | Find next / previous |
| `Esc` | Dismiss the topmost transient UI, then clear focus (priority below) |
| `0`–`9` | Go to bookmark 0–9 |
| `Ctrl+0`–`Ctrl+9` | Set bookmark 0–9 |
| `Space` | Focus/unfocus the keyboard-current row; otherwise the row at view center |
| `Shift+Space` | Pin/unpin the keyboard-current tooltip |

`Esc` has deterministic priority: close a context menu or popover, cancel an
active search, dismiss its result card or a pinned tooltip, then clear
instruction focus. One press per layer prevents a focused instruction from
disappearing when the user only meant to close a popup.

### 6.4 Jumps, bookmarks, and the command prompt

Surfer's global command prompt (fuzzy palette) gains Konata commands, mirroring
Konata's `F1` palette:

| Command | Effect |
|---|---|
| `konata_goto_row <n>` (`j <n>`) | Scroll so ID *n* is at the top-left, aligned to its fetch cycle |
| `konata_goto_rid <n>` (`jr <n>`) | Same, addressing by retire ID; prompts for thread when ambiguous |
| `konata_goto_sid <n>` | Same, addressing by simulator serial ID |
| `konata_goto_cycle <n>` | Horizontal jump to a cycle |
| `konata_find <regex>` (`f <re>`) | Find (§7.3) |
| `konata_zoom_in` / `konata_zoom_out` | Zoom |
| `konata_bookmark_set <0-9>` / `konata_bookmark_goto <0-9>` | Bookmarks |

Command history is preserved across the session (arrow keys in the prompt),
as in Konata.

**Bookmarks** store the anchor instruction identity, anchor timestamp, and zoom
rather than only screen coordinates. Ten numbered slots per source, persisted
in the state file; the *Go to bookmark* / *Set bookmark* submenus list them with
their ID/RID and cycle or time. On reload, Surfer resolves identity first and
falls back to the saved timestamp if that transaction no longer exists.
Jumping to a bookmark animates both pan and zoom.

### 6.5 Context menu and "Adjust position"

Right-click on the canvas:

* **Adjust position** — the "I'm lost" button. If the viewport has scrolled
  above, below, or away from the trace body, snap back so a valid instruction
  sits at the top-left, aligned to its fetch cycle. With synchronized tiles,
  all synced views re-align to the same `(thread ID, RID)` where possible
  (§9.3).
* **Zoom in / Zoom out** (at click point)
* **Go to bookmark ▸ / Set bookmark ▸**
* **Focus instruction / Clear focus**
* **Show in event table / Show in waveform view / Set marker here** (§10)
* **Color scheme ▸**, **Lane ▸ (Split lanes, Fixed op height)**,
  **Hide flushed ops**, **Dependency arrows ▸** — the per-tile display
  options of §8–§9.

---

## 7. Inspecting instructions

### 7.1 Hover tooltips

* **Over the canvas:** a tooltip pinned to the pointer shows
  `[cycle, ID]`, then every stage under that cell as `name[duration]`
  (overlapping lanes are comma-separated), then any per-stage annotation
  lines (`X: d:0x7 = fu(a:0x0, b:0x7), alu:0b0000…`). This is the primary way
  to read stage-level detail such as functional-unit results, cache
  hit/miss notes, or occupancy counters that the simulator attached to the
  stage event.
* **Over the label pane:** the instruction identity block — label, full
  multi-line `detail` attribute (register values, ports, addresses),
  SID/TID/RID, the FTR transaction ID (the analog of Konata's log line
  number, usable in table filters), and a "this op was flushed" notice when
  applicable.
* Tooltips are selectable/copyable (a small *Copy* affordance appears in the
  corner). Moving the pointer from the source into the tooltip keeps it open;
  `Shift+Space` pins/unpins the keyboard-current tooltip, and `Esc` closes it. This
  avoids the common failure mode where a tooltip disappears before its text
  can be selected.

### 7.2 Focused instruction

Clicking an instruction label or any of its stages **focuses** it, using the
same concept as Surfer's focused transaction. As defined in §4, a label click
also aligns fetch to the left edge; a canvas click does not pan:

* The row highlights; its stages get a bright outline.
* Its incoming and outgoing dependency relations draw emphasized (§8.1) —
  other arrows dim.
* The status strip shows its summary.
* The focus is shared state: the same transaction becomes focused in the
  waveform view and is highlighted in any open table tile, and vice versa —
  activating a row in a transaction/event table scrolls the Konata view to
  that instruction.
* Clicking empty canvas clears focus. `Esc` clears it only after transient UI
  has been dismissed in the priority order from §6.3.

### 7.3 Find

`Ctrl+F` (or `f`, or the command prompt) opens an inline find bar at the top
of the tile (not a modal):

* **Regex search** over the instruction's complete text: ID/SID/TID/RID
  numbers, label, detail, and all per-stage annotations — identical scope to
  Konata's find.
* Search runs asynchronously with a progress bar and is cancellable;
  wrap-around at the trace ends. An invalid expression shows an inline error
  and retains the last valid result set; it never clears the canvas or opens a
  modal dialog.
* On a hit, the view animates to the instruction and shows an anchored
  **result card** over unused canvas space, listing the matching lines with the
  matched substrings
  highlighted; `F3`/`Shift+F3` steps through further matches from the current
  position. The card never changes row geometry. `Esc` (or `Enter`, as in
  Konata) dismisses it.
* If the hit is currently hidden (e.g. a flushed op while *Hide flushed ops*
  is on), the card says so explicitly instead of silently failing.
* The find bar has a **"to table"** button: it converts the current pattern
  into a filtered transaction-table tile (§10.2), turning a one-at-a-time
  search into a complete result set — something standalone Konata cannot do.

---

## 8. Dependency arrows

### 8.1 What is drawn

Dependencies are FTR **relations between instruction transactions** (the
convention's `wakeup` relation, or any other relation name the producer
emits — each distinct relation name gets its own color, with a legend in the
options popover). No log-scraping heuristics are needed; if the simulator
recorded the edge, it is drawn.

Two geometries, matching Konata, selectable per tile:

* **Inside-line** (default): a straight arrow from the producer's *execute*
  stage to the consumer's *execute* stage — best for wakeup timing.
  Each endpoint uses, in order: the relation endpoint timestamp, the first
  stage whose name matches the configured execution-stage set, then the
  transaction start. A fallback endpoint is hollow and its tooltip explains
  which timestamp was unavailable; an incomplete trace therefore never
  invents an execute cycle.
* **Left-side curve:** a bezier along the left edge connecting the two ops'
  fetch positions — best for seeing dependency *distance* when zoomed out.
* **Hidden:** no arrows.

Arrows auto-hide below the dependency LOD threshold (§5.2). When *Hide
flushed ops* is active, arrows to/from hidden ops are suppressed.

### 8.2 Focus-driven exploration (beyond Konata)

* With a **focused instruction**, its direct producers and consumers are
  emphasized and everything else fades.
* **Dependency walk:** with focus active, `Alt+←` jumps to the producer (the
  one with the latest anchor timestamp if several), `Alt+→` to the earliest
  consumer. Repeated presses cycle through ties in stable ID order.
* **Producer-chain highlight:** from the context menu of a focused
  instruction, *Highlight producer chain* transitively marks recorded producer
  relations backwards until an instruction has no in-view producer. This is a
  dependency-ancestry aid, not a claim that the highlighted path is the
  simulator's timing-critical path.

---

## 9. Color, comparison, and large-scale reading

### 9.1 Color schemes

Selectable per tile (context menu / options popover):

| Scheme | Meaning of color |
|---|---|
| **Auto** (default) | Hue derived from the stage's position in the pipeline (order of first appearance), so `F` → `Dc` → … → `Cm` forms a stable rainbow; lanes shift the palette so lane 1 is visibly distinct |
| **Unique** | Hue per distinct stage name (stable across lanes) |
| **Thread ID** | Hue per `thread_id`; stage depth modulates lightness — the scheme for SMT traces |
| **Flat colors** (Orange, RoyalBlue, …) | Whole trace in one color — meant for overlay comparison of two traces |
| **Custom…** | User-defined schemes in the Surfer config: per lane, per stage name, HSL with `auto` wildcards. Custom schemes appear in the menu alongside built-ins |

Universal rules: stages named as stalls (`f`, `stl` — set configurable) render
gray in every scheme; flushed ops are overlaid dark; colors come from the
active Surfer theme so light and dark modes both stay readable. Stall matching
is case-sensitive by default (`f` does not recolor fetch stage `F`). Every
scheme meets the theme's contrast target, focused/flush/warning states use
outlines or glyphs as well as color, and the options popover includes a
color-vision-deficiency-safe palette.

### 9.2 Hide flushed ops

A per-tile toggle that removes squashed instructions from the Y axis entirely
(remaining rows close the gaps but retain their original fetch order). The
wrong-path noise disappears and committed throughput becomes the visible
slope. Original ID and RID labels do not change. The view keeps the
top-of-screen instruction stable when toggling, find warns when a hit is hidden,
and dependency arrows to hidden ops are suppressed. Instructions marked
non-flushed but lacking RID remain visible in place with an `unretired/unknown`
warning badge rather than being guessed as committed or silently removed.

### 9.3 Comparing two traces

The core Konata workflow "run A/B, load both, overlay them" maps to tiles:

* **Side-by-side (default):** open each trace's Konata tile, drag one next to
  the other, enable **Synchronize scroll** on both (context menu). Synced
  tiles share zoom, keep their splitters aligned, and anchor Y to the same
  `(thread ID, RID)`. Horizontal movement preserves each anchor instruction's
  relative fetch-cycle offset; it does not assume the two traces use the same
  absolute timestamp.
* **Overlay mode:** choose *Compare as overlay…* and select another Konata tile
  (or drag a tile onto another while holding the platform's alternate-action
  modifier). The combined tile names both sources. The front trace renders at
  about 50% opacity over the back trace; give each a flat color scheme (e.g.
  Orange vs. RoyalBlue) and divergence between runs shows up as color fringes.
  Holding the pointer down on either trace emphasizes that trace temporarily,
  matching Konata's drag-to-emphasize workflow.
* Synchronization uses `(thread_id, retire_id)` only when the key is present
  and unique in both traces. The tile header reports unmatched and duplicate
  keys. For traces without comparable RIDs, the user may explicitly align by
  stable fetch ID or timestamp; Surfer never silently chooses a weaker key.

### 9.4 The minimap (beyond Konata)

An optional thin overview strip along the right edge of the canvas shows the
whole trace vertically compressed (the §5.2 "skyline" rendering) with the
current viewport as a draggable lens. Flush regions and long stalls are
visible in the minimap directly, making "jump to the next trouble spot" a
single click. Toggle from the options popover.

---

## 10. Integration with the rest of Surfer

This is where the integrated Konata view outgrows the standalone tool.

### 10.1 Shared time: cursors and markers

The Konata X axis is always trace time; a configured pipeline clock adds a
cycle-number projection over it. Surfer's cursor and markers are therefore
meaningful in either ruler mode:

* The **primary cursor** renders as a vertical line on the Konata canvas at
  its timestamp; clicking the cycle ruler moves it. Moving the cursor in the
  waveform view moves it in Konata and vice versa — *the* mechanism for "the
  pipeline hiccupped here; what were the signals doing?"
* **Markers** (named/numbered) appear as pins on the cycle ruler; the
  standard marker commands work while a Konata tile is focused. Right-click →
  *Set marker here* drops one at the clicked cycle.
* A stage box's context menu offers *Move cursor to stage start/end*.

### 10.2 Tables ⇄ Konata

* **Instruction table:** the existing per-generator transaction table shows
  instructions with `label`, `retire_id`, `flushed`, … as columns — sort by
  duration to find the slowest ops, filter `flushed == true`, etc.
  **Activating a row scrolls every open Konata view to that instruction** and
  focuses it.
* **Event table:** the existing FTR event table lists stage events with the
  parent instruction; activating a row navigates to the parent and highlights
  the specific stage.
* From Konata to tables: context-menu *Show in event table* / find-bar *to
  table* create pre-filtered table tiles.

### 10.3 Waveform view

* *Show in waveform view* on an instruction scrolls the waveform/transaction
  view to that transaction's time and focuses it — useful when the same FTR
  file (or a companion VCD/FST loaded side by side) carries signal data.
* Conversely, focusing a pipeline-trace transaction in the waveform view
  offers "Show in Konata view" in its context menu.

### 10.4 Remote traces, reload, and scripting

* Konata views target the same behavior for **remote FTR traces served by
  Surver** as for local files, including progressive loading and cancellation.
  If the connected Surver does not expose the required transaction/event
  data, the entry point is disabled with a version/capability explanation
  rather than failing after the tile opens.
* **Reload** (`Ctrl+R` / toolbar) re-reads the file in place, preserving the
  focused transaction and viewport by stable identity when possible, then by
  timestamp. It keeps zoom, bookmarks, and options. When Surfer's file watcher detects
  the trace changed on disk (a simulation re-run), the standard non-modal
  reload suggestion appears. Ongoing simulations can be reloaded to watch a
  trace grow; the progress indicator distinguishes newly appended data from a
  complete replacement.
* **WCP:** the Waveform Control Protocol gains the Konata navigation verbs
  (open view, goto row/rid/cycle, set bookmark, focus instruction), so
  external tooling — regression triage scripts, IDE plugins pointing at a
  failing instruction — can drive the view programmatically.
* **Drag & drop** an `.ftr` file onto Surfer as usual; if it contains a
  conforming pipeline trace the §3 suggestion appears.

---

## 11. Pipeline statistics

*Trace → Pipeline statistics…* (menu, context menu, or
`konata_stats` command) computes summary statistics over the instruction
generator and presents them in a standard **table tile** (filterable,
sortable, copyable — Konata's modal dialog with a filter box, upgraded):

* Fetched ops, committed ops, elapsed cycles, and **IPC**. For whole-trace
  statistics, elapsed cycles span from the first instruction start through the
  end of the last committed instruction, using half-open cycle coverage;
  committed means `flushed == false` with a valid RID. The table shows
  `unknown` rather than guessing when retirement or clock metadata is missing.
* Flush count and flushed-op count, plus cause attribution when recorded or
  explicitly estimated, giving misprediction rates and **MPKI** for branch,
  jump, and memory speculation.
* Per-stage aggregates (unique to Surfer, since stages are structured
  events): average and max duration per stage name, total stall cycles per
  stage — the "where do cycles go" table.
* Per-thread breakdowns when `thread_id` varies.

Flush attribution first uses an explicit cause relation or cause-transaction
attribute from the pipeline convention. Only when that is absent may the view
use the preceding committed instruction as an estimate; estimated rows carry
an `estimated` badge and are excluded from exact-rate totals unless the user
opts in. Classification of branches/jumps/stores uses an explicit
instruction-class attribute when present, otherwise the same pluggable label
heuristics as Konata (generic and x86-gem5 patterns built in). The table exposes
which classifier produced each row. Statistics computation is asynchronous
with progress and never blocks the canvas. Because results land in a table
tile, they can sit docked next to the pipeline while you work, instead of
living in a modal.

For statistics *over a region*, select a cycle/time range on the ruler
(click-drag) and choose *Statistics for selection* — IPC and stall
distributions for just that window, ideal for comparing loop iterations. A
region uses `[selection start, selection end)`: IPC counts instructions whose
retirement timestamp falls inside it; fetched counts use start timestamps;
stage durations are clipped to the selection before aggregation. The results
header repeats these rules and the exact selected time range.

---

## 12. Options, theming, and persistence

* **Options popover** (gear icon in the tile header): color scheme, lane
  mode, hide-flushed, dependency-arrow style, minimap, ruler unit, and the
  four LOD thresholds (text / frames / colors / arrows) plus zoom step
  granularity. Pipeline clock period/phase and comparison alignment are also
  shown when relevant. These are Konata's Settings concepts scoped per tile,
  with global defaults in Surfer's config.
* **Themes:** the view derives all chrome (backgrounds, stripes, invalid
  regions, fonts) from the active Surfer theme; both light and dark themes
  ship tuned pipeline palettes. Custom color schemes live in the user config
  alongside Surfer's existing theme overrides.
* **Persistence:** everything a user set — open Konata tiles, per-tile
  options, splitter positions, scroll/zoom, bookmarks, sync groups — is part
  of the Surfer state file, so a saved session reopens exactly as left,
  including for state files checked into a repo to share "look at this flush
  storm" reproductions with teammates.
* **Snapshot friendliness:** since the view is deterministic for a given
  state file, golden-image snapshot tests cover it the same way as the other
  views.

---

## 13. Robustness and accessibility

### 13.1 Data-quality states

Malformed data is visible but never allowed to corrupt the projection:

| Condition | User-visible behavior |
|---|---|
| Event has no incoming `parent_of` | Omitted from instruction rows; counted as an orphan in the quality badge and available in the raw event table |
| Event has multiple candidate parents | Attached to the first relation in recorded order, marked with a warning; all candidate relations remain inspectable |
| Event lacks `name` | Rendered as `<unnamed stage>` with a warning outline |
| Event lies outside its parent range | Rendered at its recorded time with a warning outline; never clipped or discarded |
| End precedes start | Rendered as an invalid point at start time and reported as malformed; negative duration is never displayed |
| Instruction begin time moves backward | Recorded fetch order and ID are preserved; the affected row is marked and remains at its recorded X position |
| Unknown lane value | Assigned a stable textual lane key; the original attribute remains visible |
| Missing/duplicate RID | Fetch-order view remains available; RID jumps, committed-status statistics, and RID synchronization explain the ambiguity |
| Missing clock mapping | Trace-time ruler remains usable; cycle-only commands and IPC explain how to configure the clock |

The tile header shows a non-modal quality badge with counts by category. Its
popover links to filtered raw tables. Warnings are deduplicated, persist across
pan/zoom, and update during progressive loading. A fatal parse error keeps all
successfully loaded rows visible, marks the trace incomplete, and offers the
parser diagnostic and byte/time position.

### 13.2 Keyboard and assistive technology

Every context menu can be opened and operated from the keyboard; essential
navigation and inspection actions are also reachable from the command prompt.
Tab moves between tile controls, ruler, label list, and canvas; arrow keys
operate the focused region. The label pane exposes a virtualized list whose
accessible name contains ID, optional SID/TID/RID, label, flushed state, and
stage count. Stage boxes expose name, start, end, duration, lane, parent
instruction, and warning state. Focus is always visible, never indicated by
color alone, and follows the same stable transaction identity exposed to tables
and waveform view.

Text and controls honor Surfer's UI scale. At large text sizes the label pane
wraps or truncates with an accessible full label; the canvas does not overlap
controls. Reduced-motion disables animated pans, zooms, and opacity transitions.

### 13.3 Completion criteria

The target UX is complete when every row of the parity matrix has an automated
behavior or snapshot test, malformed cases above are covered, local and remote
capability failures produce the specified states, and the same saved state
restores stable focus/alignment after reload. Performance acceptance uses
representative small, million-instruction, and multi-million-instruction traces;
the measured trace sizes and interaction latency are recorded with results
rather than asserted in this document.

---

## 14. Usage scenarios (walkthroughs)

### 14.1 First look at a new core

1. Drag `dhrystone_rsd.ftr` into Surfer → "Pipeline trace detected" → open
   Konata view.
2. Zoom all the way out: the skyline shows a dense start, a long thin gash at
   ~cycle 90k, dark wedges every few thousand cycles.
3. Drag the minimap lens to the gash, zoom in: a 600-cycle `Sc` box on one
   load — scheduler starvation behind a cache miss. Hover the stage: the
   simulator's annotation says `dcache miss, MSHR full`.
4. `Ctrl+0` to bookmark it; *Set marker here*; save the session state file
   and attach it to the bug report.

### 14.2 Branch-flush triage

1. Open *Pipeline statistics* — branch MPKI is 14, unusually high.
2. Leave **Hide flushed ops** off (the default) and zoom to mid-trace: repeated
   dark triangles ~40 instructions tall.
3. Click the branch at a triangle's tip to focus it; the label pane shows
   `bne a5, a4, -0x14` — the loop back-edge.
4. Find bar → pattern `bne a5` → *to table*: a filtered table of all 2,310
   occurrences; sort by duration; activate the worst row — Konata view jumps
   there.
5. Compare with a second run (new predictor): open its trace, sync scroll,
   flat Orange vs. RoyalBlue overlay — the triangles vanish in run B.

### 14.3 Dependency-chain analysis

1. A hot loop achieves IPC 1.1 on a 4-wide core. Zoom to one iteration.
2. Click the last instruction of the iteration; *Highlight producer chain*:
   a 9-instruction recorded dependency chain lights up across the iteration.
3. Walk it backwards with `Alt+←`, reading each producer's stage annotations;
   the chain pivots on a `mul` whose `X` stage is 4 cycles.
4. Conclusion in minutes: the loop is latency-bound on the multiply chain,
   not fetch- or issue-bound — visible as arrows spanning exactly the
   iteration length.

### 14.4 SMT interleave inspection

1. Load a 2-thread trace; color scheme → **Thread ID**.
2. The canvas becomes two interleaved color families; fetch bandwidth
   arbitration is directly visible as the alternation pattern.
3. Filter one thread into its own event-table tile for counts; use markers to
   delimit a region and *Statistics for selection* for per-thread IPC there.

### 14.5 Beyond CPUs

A NoC simulator emits one transaction per packet and one event per hop
(`name = router ID`, `lane = virtual channel`). The Konata view then shows
packets as rows and route progress as stage boxes — congestion appears as the
same vertical cliffs, and `wakeup`-style relations can encode
credit-dependency between packets. The same convention-driven view applies
without CPU-specific UI behavior.

---

## 15. Konata feature parity matrix

Target coverage for every user-visible Konata feature and where it lands in
Surfer:

| # | Konata feature | Surfer Konata view |
|---|---|---|
| 1 | Kanata log format input | Via `konata2ftr` conversion to FTR (LWTR4SC tool); native FTR is the on-disk format |
| 2 | gem5 O3PipeView (+O3CPUAll) input | Via conversion to FTR; dependency/annotation data maps to relations & event labels |
| 3 | Gzip input, drag & drop, file dialog, recent files, CLI args | Surfer's standard loading paths (drag & drop, dialog, recents, CLI, URLs) |
| 4 | Progressive load with progress bar; browsable during load | §5.1 |
| 5 | Reload (`Ctrl+R`), file-change detection prompt | §10.4 |
| 6 | Multiple traces in tabs; next/prev tab; middle-click close | Tiles & tab groups; standard Surfer tile management |
| 7 | Label pane (`ID: s.. (t..: r..): label`), click-to-align, tooltip | §4 ① |
| 8 | Splitter between panes, persisted, synced across compared traces | §4 ② |
| 9 | Stage boxes with names, cycle numbering `1 2 3…`, gradient, borders, stripes | §5.1 |
| 10 | Out-of-range regions shaded | §5.1 |
| 11 | Flushed-op dark overlay | §5.1 |
| 12 | Zoom levels (2^level, ~24 steps), zoom at pointer, animated, zoom-step config | §5.2, §6 |
| 13 | LOD thresholds (text/frame/detail/dependency), configurable | §5.2, §12 |
| 14 | Diagonal-following vertical scroll; Shift to disable | §6.1 |
| 15 | Wheel/drag/double-click/keyboard navigation incl. PageUp/Down | §6.2–6.3 |
| 16 | "Adjust position" recovery | §6.5 |
| 17 | Command palette (`F1`): `j`, `jr`, `f`, `l`; history | Surfer command prompt (§6.4); `l` = Surfer's normal file-open commands |
| 18 | Find: regex over labels/details/stage labels, F3/Shift+F3, result popup with highlighted matches, wraparound, async + cancellable, hidden-hit notice | §7.3 |
| 19 | Bookmarks ×10 (position+zoom), keys `0-9`/`Ctrl+0-9`, persisted, menus | §6.4 |
| 20 | Tooltips: pipeline `[cycle,id] stage[len]` + stage labels; label-pane details | §7.1 |
| 21 | Dependency arrows: inside-line, left-side curve, hidden; wakeup type | §8.1 |
| 22 | Color schemes: Auto, Unique, ThreadID, flat (Orange/RoyalBlue), custom user schemes; stall stages gray | §9.1 |
| 23 | Split lanes (`n`), fixed op height | §5.3 |
| 24 | Hide flushed ops (stable identities, compacted fetch order, find/deps interplay) | §9.2 |
| 25 | Overlay comparison + emphasize-on-hold; synchronized scroll; explicit alignment keys | §9.3 |
| 26 | Stats dialog (ops/cycles/IPC, flush causes, Br/Jump/Mem miss rates & MPKI, filter box; ISA heuristics) | §11, upgraded to a table tile |
| 27 | Settings dialog (thresholds, zoom factor) with search | §12 options popover + config |
| 28 | Light/dark UI themes; styleable pipeline palette | §12, Surfer themes |
| 29 | Per-stage labels (Kanata `L` type 2 / gem5 ExLog) shown in tooltips | §7.1 (event `label` attributes) |
| 30 | Window/layout persistence | Surfer state files (§12) |

Konata capabilities intentionally *not* carried over: the built-in developer
tools toggle (Electron-specific) and modal error dialogs (Surfer uses its
standard non-modal error toasts/log).

### New capabilities beyond Konata (summary)

1. Cross-view navigation: shared focused transaction, cursors/markers on the
   pipeline ruler, jump between Konata ⇄ table ⇄ waveform (§10).
2. Find-to-table: turn a search into a complete filtered result set (§7.3).
3. Dependency walking (`Alt+←/→`) and transitive producer-chain highlight
   (§8.2).
4. Region statistics via ruler selection; per-stage stall aggregates;
   per-thread breakdowns (§11).
5. Minimap overview strip with click-to-jump (§9.4).
6. Cycle ruler with explicit clock mapping and real time units (§4 ③).
7. Remote traces (Surver), WCP scripting, state-file reproducibility,
   snapshot-testable rendering (§10.4, §12).
8. Copyable tooltips (§7.1).
9. Format-agnostic by convention: any "items through phases" trace, not just
   CPUs (§14.5).

---

## 16. Out of scope (for this document)

* Implementation architecture, rendering pipeline, caching strategy.
* The FTR pipeline-trace convention specification itself (see Surfer's
  [FTR event convention](docs/development/FTR_EVENTS.md), LWTR4SC's
  `FTR_EVENTS.md`, and the `konata2ftr` tool documentation).
* Writing converters for additional simulator log formats.
