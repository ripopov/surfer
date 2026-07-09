# FTR-events performance fixes

This fork adds performance fixes required by the FTR transaction-**events**
convention (see `surfer/docs/development/FTR_EVENTS.md`).

## Problem

The events convention represents every in-transaction event as its own
transaction linked to its parent by a `parent_of` relation, so a trace ends up
with roughly as many relations as transactions. The parser attached relations
by scanning the entire relation list once per transaction — `O(transactions ×
relations)`. With millions of events that is ~`O(n²)`, and large traces became
effectively unloadable (a 96k-transaction trace took ~7 s; a 1M-event trace
would take many minutes).

`cargo run --release --example events_bench -- <num_instructions>` reproduces
and benchmarks this (it synthesizes a trace modeled on LWTR4SC's
`lwtr_event_example_wait.cpp`). After the fixes, the same 96k-transaction trace
loads in ~30 ms and an 800k-transaction / 700k-relation trace in ~0.34 s.

## Why the API changed

Making relation attachment linear means relations are stored **once** and
referenced by index instead of being cloned into every transaction. Two further
fixes on the hot paths required type changes that are visible in the public API:
timestamps are read as `u64` (the FTR writer only emits `uint64_t`), and
repeated strings are interned so a million identical event names are stored
once rather than a million times. These could not be done without changing the
types that consumers touch.

## API changes

| Before | After | Why |
|---|---|---|
| `Transaction::inc_relations` / `out_relations`: `Vec<TxRelation>` | `Vec<usize>` indices into `FTR::tx_relations`, resolved via new `FTR::get_relation(i)` | Store each relation once; attach in `O(1)` via prebuilt source/sink index maps |
| `Event::start_time` / `end_time`, `get_start_time()` / `get_end_time()`, `FTR::max_timestamp`: `BigUint` / `BigInt` | `u64` | Remove per-comparison heap clones on the render binary-search path; drops the `num-bigint` dependency |
| `Attribute::name`, `TxRelation::name`, `DataType::{Enumeration,BitVector,LogicVector,String}`, `FTR::str_dict` values: `String` | `Arc<str>` (interned, shared with the dictionary) | Avoid storing millions of duplicate attribute/relation name and value strings |

Migration notes for consumers (e.g. Surfer):
- Resolve relations with `ftr.get_relation(idx)` instead of indexing
  `Vec<TxRelation>` directly.
- Timestamps are plain `u64`; drop `BigUint`/`BigInt` conversions.
- Compare interned strings via `name.as_ref() == "..."` (e.g.
  `rel.name.as_ref() == "parent_of"`). The serde `rc` feature is enabled so
  `Arc<str>` fields still (de)serialize.

The 3-element relation fallback was also fixed: it now resolves the sink
stream by the sink transaction id (previously both stream lookups used the
source id).
