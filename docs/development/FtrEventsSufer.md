# FTR Transaction Events in Surfer — UX Design

This document designs the user experience of first-class FTR transaction event
support in Surfer. It builds on the convention defined in
[FTR_EVENTS.md](FTR_EVENTS.md): an *event* is a transaction written through a
`<parent_generator_name>.events` generator and linked to exactly one parent
transaction by an incoming `parent_of` relation. Events are typically
zero-duration and carry their name in a `BEGIN` attribute called `name`.

This is a UX specification only. It deliberately avoids implementation detail
and instead defines what the user sees, what they can do, and what happens
when they do it.

## Design Goals

1. **Events read as annotations, not as more transactions.** The user's mental
   model is "this instruction stalled at t=112 because of a data hazard", not
   "there is a second transaction related to the first one". The UI should
   present events *inside the context of their parent* wherever possible.
2. **Zero configuration for conforming traces.** Opening a trace produced by
   LWTR4SC `record_event` should give good event UX immediately — no toggles,
   no setup.
3. **Graceful degradation, never data loss.** Malformed or non-conforming
   traces (events without parents, out-of-range timestamps, multiple parents)
   must still display all data, falling back to today's plain-transaction
   rendering.
4. **Power users keep raw access.** The `.events` generator remains a real
   generator: it can still be displayed as its own row, opened in a table, and
   inspected transaction-by-transaction. Event semantics are a presentation
   layer, not a filter.
5. **Scales to millions of events.** Every interaction below must remain
   meaningful when a parent generator has 1M transactions and 5M events —
   dense views aggregate rather than degrade.

## Terminology Used in the UI

- **Event** — a transaction from a `.events` generator with exactly one
  incoming `parent_of` relation. Shown to users with the word "event".
- **Parent transaction** — the source of that relation.
- **Event marker** — the diamond glyph drawn for an event (Surfer already
  renders zero-duration transactions as diamonds).
- **Orphan event** — a transaction in a `.events` generator with no incoming
  `parent_of` relation. Rendered as an ordinary transaction per the
  convention, but labeled "orphan event" in details/tooltips so the user
  understands why it gets no special treatment.

Surfer detects events automatically using the convention's discriminator:
generator name ends in `.events` *and* a sibling generator with the matching
base name exists in the same stream. A generator that merely happens to be
named `foo.events` with no sibling `foo` is treated as an ordinary generator.

---

## 1. Hierarchy Sidebar

### What the user sees

In the stream/generator hierarchy, a parent generator that has a matching
`.events` sibling shows an event badge after its name:

```text
▼ cpu0
    instruction          ⚡ 3 421 events
    bus_access
```

- The `.events` generator itself is **not listed as a separate top-level
  entry by default** — it is folded into its parent's presentation. This
  keeps the hierarchy matching the writer's intent (one logical generator
  with annotations).
- A view menu / sidebar toggle **"Show raw event generators"** restores
  today's behavior and lists `instruction.events` as its own entry (dimmed,
  with the ⚡ icon). Power users use this to add the raw event row or open a
  raw event table. The toggle is persisted in the session state file.
- Orphan-only `.events` generators (no matching parent generator, or no
  conforming relations at all) are always listed as ordinary generators.

### What the user can do

- **Double-click / "Add" the parent generator** — adds the parent generator
  as a displayed row *with events overlaid* (see §2). This is the headline
  zero-configuration behavior: one action shows transactions and their
  events together.
- **Right-click the parent generator** gains event entries in the context
  menu:
  - *Show transactions in table* (existing)
  - *Show events in table* — opens an event table for this generator (§4)
  - *Add events as separate row* — adds the `.events` generator as its own
    waveform row, today's behavior, for users who want events on a dedicated
    lane
- **Adding a whole stream** adds each parent generator with events overlaid;
  `.events` generators are not added as duplicate rows.

---

## 2. Waveform View — Events Overlaid on the Parent Row

This is the core of the design: events render **on top of the parent
generator's transaction bars**, in the same row, at their recorded time.

```text
              ┌─────────────────────────────────┐
 instruction  │ LW   ◇        ◆      ◇          │   ◇ = event marker
              └─────────────────────────────────┘
                   fetch     stall   writeback
```

### Marker placement and appearance

- Each event draws as a small diamond centered vertically on the lane of its
  **parent transaction** (not on the event generator's own lane assignment),
  horizontally at the event's start time. The existing diamond glyph,
  `transaction_event` theme color, and background-colored outline are reused
  so events are visible on top of same-colored bars.
- Events of one parent that overlap in time (concurrent stalls) stack as
  diamonds offset vertically within the parent's bar height; if more than
  fit, they cluster (see *Density* below).
- Short-duration (non-zero) events draw as a thin bracket/underline spanning
  their duration along the bottom edge of the parent bar, with the diamond at
  the start. The duration is always visible in the tooltip and details panel.
- **Out-of-range events** (recorded time outside the parent's span, allowed
  by the convention) still render at their recorded time, on the parent's
  lane, but as a **hollow diamond** with a warning tint. The tooltip and
  details panel state: "outside parent transaction range".

### Density / zoomed-out behavior

At low zoom, hundreds of events can map to one pixel column. Markers
aggregate into a **cluster glyph**: a slightly larger diamond with a count
badge (e.g. `◆₁₂`). Hovering a cluster shows the breakdown by event name
("stall ×9, retry ×3"); clicking it zooms the viewport so the cluster
resolves into individual markers. The user never sees a smear of overdrawn
diamonds, and never loses the information that "something happened here".

### Per-row event display mode

Each displayed row whose generator has events carries a three-way display
mode, set from the row's right-click menu and persisted in the state file:

- **Overlay** (default) — markers on the parent bars, as above.
- **Separate row** — events appear as an attached sub-row directly below the
  parent row (visually grouped with it, indented name `instruction ⚡events`).
  Useful when overlays obscure short parent transactions.
- **Hidden** — parent transactions only; the row shows a muted ⚡ in its name
  area so the user can tell events exist but are hidden.

The same three-way control appears in the row context menu for whole-stream
rows. In whole-stream view, events are *always* drawn as overlays on their
parent's lane rather than occupying their own generator lane — this resolves
the existing collision where a parent and a concurrent event could be drawn
on top of each other.

### Tooltips

- **Hovering an event marker** shows an event tooltip: event name (large,
  first line), time, duration if non-zero, the event's own attributes, and
  one summary line for the parent ("in instruction tx#1000, 100–130 ns").
- **Hovering a cluster** shows the per-name counts and the time span covered.
- **Hovering a parent bar** appends one line to the existing transaction
  tooltip: "events: 3 (operand_fetch, stall, writeback)" — names truncated
  with "+N more" past three.

---

## 3. Selection, Focus, and the Details Panel

### Clicking an event marker

- The event becomes the focused transaction (existing focus machinery), drawn
  in the focus highlight style.
- Its **parent transaction is co-highlighted** with a soft outline so the
  pair reads as one unit. No bezier relation arrow is drawn for the
  `parent_of` link when the event is overlaid on its parent — an arrow from a
  bar to a diamond inside the same bar is noise. (Arrows still draw when the
  event is on a separate row, and for all non-event relations.)
- The right-side details panel renders an **Event** card instead of the
  generic transaction card:

  ```text
  Event: stall                          (hollow-diamond ⚠ badge if out of range)
  Time: 112 ns   Duration: 3 ns
  ───────────────────────────────
  reason      data_hazard
  cycles      3
  ───────────────────────────────
  Parent: instruction tx#1000  [Go to parent]
    pc       0x80000000
    opcode   LW
  ```

  The event's `name` attribute is promoted to the title; remaining attributes
  list below. The parent section shows the parent's identity and `BEGIN`
  attributes, with **Go to parent** focusing the parent transaction (and
  scrolling/zooming it into view if needed).

### Clicking a parent transaction

The existing focused-transaction details panel (properties, attributes,
relations) gains an **Events** section listing this transaction's events in
time order:

```text
Events (3)
  108 ns  operand_fetch   operand=0, register_bank=2
  112 ns  stall           reason=data_hazard, cycles=3
  120 ns  writeback
```

- Clicking a row focuses that event (marker highlights in the waveform).
- Double-clicking additionally moves the cursor to the event's time.
- `parent_of` relations that point to events are **omitted from the generic
  Outgoing Relations table** (they are represented by the Events section
  instead); all other relations list as today, now also showing the relation
  *name* alongside source/sink.

### Keyboard navigation

- Existing `transaction_next` / `transaction_prev` continue to move between
  parent transactions and skip overlaid events.
- New `event_next` / `event_prev` commands: with a parent focused, focus its
  first/last event; with an event focused, move to the next/previous event of
  the same parent, wrapping into neighboring parents' events at the ends.
  Available in the command palette and bindable.
- `Esc`/unfocus behaves as today.

---

## 4. Event Tables

The table subsystem gains an **event table** model, opened from the
hierarchy context menu (*Show events in table*) or from a button in the
focused parent's Events section header (*Open in table*, which opens the
generator-wide event table pre-filtered to that parent).

Columns:

| time | duration | name | parent | …event attributes… |

- **name** is the promoted `name` BEGIN attribute — a first-class, sortable,
  filterable column rather than one of many attribute columns.
- **parent** shows the parent transaction's id and generator (e.g.
  `instruction #1000`); clicking it focuses the parent in the waveform.
  Orphan events show "—" here.
- Attribute columns are discovered from the events as in existing transaction
  tables; standard sorting and filtering apply. Filtering by `name` is the
  expected primary workflow ("show me only `stall` events"), so the name
  column header offers a quick value-picker of observed event names with
  counts, in addition to free-text filtering.
- Row interaction matches existing transaction tables: selecting a row
  focuses the event in the waveform and brings it into view.
- The table opens as a tile in the existing tile layout (first table splits
  below the waveform, further tables split horizontally), and is captured in
  the session state file like other tables.

The plain *Show transactions in table* on a parent generator is unchanged,
but gains an **events** count column (clicking a count opens the event table
filtered to that parent). The raw `.events` generator can still be opened as
an ordinary transaction table via "Show raw event generators".

---

## 5. Edge Cases and Violation Handling (User-Visible Behavior)

Per the convention's rules, Surfer never discards event data:

| Situation | What the user sees |
|---|---|
| Event with no `parent_of` parent (orphan) | Rendered as an ordinary transaction of the `.events` generator (its own row/lane if that generator is displayed). Tooltip/details label it "orphan event". Not overlaid on anything. |
| Event with multiple incoming `parent_of` relations | Treated as a child of the first parent. Details panel shows a notice: "multiple parents recorded; showing first". The trace-level status bar shows a one-time warning badge for the file. |
| Event outside parent's time range | Rendered at its recorded time as a hollow warning-tinted diamond; tooltip and details say "outside parent transaction range". Never moved, never hidden. |
| `parent_of` relation whose sink is not in a `.events` generator | Ordinary hierarchy relation — arrows and relation tables as today, no event treatment. |
| `.events` generator with no matching parent generator | Ordinary generator; no badge, no folding, no event UI. |
| Non-zero-duration events | Fully supported: bracket rendering in overlay mode, duration column in tables and details. |

---

## 6. Settings, Theming, and Persistence

- **Theme**: existing `transaction_event` color is kept as the default marker
  color. New theme keys: cluster badge color, out-of-range warning tint, and
  parent co-highlight outline. Optionally, themes may enable deterministic
  per-event-name coloring (hash of the name → palette entry) so `stall` and
  `operand_fetch` are visually distinguishable at a glance; off by default to
  keep traces calm.
- **Per-row state** (overlay/separate/hidden mode) and the sidebar's "show
  raw event generators" toggle persist in the `.surf.ron` state file, so a
  saved session restores exactly the same view.
- **Global config**: one switch `ftr_events_enabled` (default on) disables
  all event semantics and reverts to today's plain rendering — an escape
  hatch for non-conforming traces that accidentally match the naming
  convention.
- User-assigned item colors continue to override theme colors, for both
  parent rows and separate event rows.

---

## 7. Primary User Workflows (End-to-End)

**W1 — "Why did this instruction take so long?"**
Open trace → add `instruction` generator (events overlay automatically) →
spot a bar with several diamonds → hover: "stall, reason=data_hazard" →
click the diamond → details panel shows cycles=3 and the parent's pc/opcode →
*Go to parent* to read the full instruction record.

**W2 — "Find all L2 misses across the run."**
Right-click `instruction` → *Show events in table* → click the `name` column
quick-picker → choose `l2_miss (1 204)` → table filters → click rows to jump
the waveform to each occurrence; sort by an attribute column (e.g. `latency`)
to find the worst one.

**W3 — "What's the event mix in this hot region?"**
Zoomed out, the user sees cluster diamonds with counts over a busy region →
hover for the per-name breakdown → click a cluster to zoom in until
individual markers resolve → step through them with `event_next`.

**W4 — power user, non-conforming trace.**
Toggle *Show raw event generators* → add `instruction.events` as its own
row → events appear on dedicated lanes exactly as any generator does today →
open the raw transaction table for it. Nothing about the convention is forced
on them.

---

## 8. Out of Scope (for this UX phase)

- Implementation strategy, data structures, and the `ftr_parser` performance
  fixes listed in FTR_EVENTS.md (prerequisites, but invisible to users except
  as load-time speed).
- Analytics on top of events (histograms of stall reasons, per-event-name
  statistics panels). The event table's filter/sort covers the first need;
  aggregate views are a natural follow-up once tables and overlays exist.
- WCP protocol extensions for driving event focus remotely.
- Editing or authoring events from within Surfer.
