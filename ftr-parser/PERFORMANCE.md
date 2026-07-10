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
referenced by index instead of being cloned into every transaction. Two sorted
permutation arrays provide source and sink lookups without a per-transaction
hash-map allocation. Two further fixes on the hot paths required type changes
that are visible in the public API: timestamps and all on-disk identities are
read as `u64`, and repeated strings are interned so a million identical event
names are stored once rather than a million times. These could not be done
without changing the types that consumers touch.

## API changes

| Before | After | Why |
|---|---|---|
| `Transaction::inc_relations` / `out_relations`: `Vec<TxRelation>` | `Vec<usize>` indices into `FTR::tx_relations`, resolved via `FTR::get_relation(i)` | Store each relation once; attach in `O(log n + k)` through sorted source/sink permutations |
| `Event::start_time` / `end_time`, `get_start_time()` / `get_end_time()`, `FTR::max_timestamp`: `BigUint` / `BigInt` | `u64` | Remove per-comparison heap clones on the render binary-search path; drops the `num-bigint` dependency |
| Stream, generator, transaction, and dictionary IDs: `usize` | Dedicated `u64` newtypes | Preserve FTR identity exactly and make native/wasm state files portable |
| `Attribute::name`, `TxRelation::name`, `DataType::{Enumeration,BitVector,LogicVector,String}`, `FTR::str_dict` values: `String` | `Arc<str>` (interned, shared with the dictionary) | Avoid storing millions of duplicate attribute/relation name and value strings |
| Transaction chunks: opaque `(offset, compressed)` pairs | `BlockMeta` directory plus `FTR::visit_stream_blocks` | Expose byte ranges, time bounds, validation status, and block-at-a-time decode without retaining the generic transaction graph |
| Relationship chunks: decoded eagerly into `Vec<TxRelation>` | `RelationBlockMeta` plus block visitors/random access | File open records offsets and sizes only; compact projections consume one relation batch at a time |

Migration notes for consumers (e.g. Surfer):
- Resolve relations with `ftr.get_relation(idx)` instead of indexing
  `Vec<TxRelation>` directly.
- Timestamps are plain `u64`; drop `BigUint`/`BigInt` conversions.
- Compare interned strings via `name.as_ref() == "..."` (e.g.
  `rel.name.as_ref() == "parent_of"`). The serde `rc` feature is enabled so
  `Arc<str>` fields still (de)serialize.
- Use the ID newtypes directly; convert to `usize` only at a checked container
  indexing boundary.
- Use `TxStream::tx_blocks` for progress and random-access metadata. The legacy
  offset list is private compatibility state.

`FTR::visit_stream_blocks` decodes file-backed chunks in recorded order and
drops each batch after the callback returns. It updates every `BlockMeta`
status to `Loaded` or `Error`; `load_stream_into_memory` reports the same
statuses while populating generator bodies. The visitor is the bounded-input
primitive used by compact projections and does not mark the generic stream as
resident.

The 3-element relation fallback was also fixed: it now resolves the sink
stream by the sink transaction id (previously both stream lookups used the
source id).

File-backed relations are now lazy. `parse_ftr` retains one small
`RelationBlockMeta` per encoded chunk, while `visit_relation_blocks` decodes a
single batch and drops it after the callback. The Konata normalizer collapses
`parent_of` records immediately to a first-parent row plus a count and retains
only candidate dependency edges. Legacy whole-stream loading still calls
`load_relations_into_memory` and builds the two permutation indexes, then may
release them again once all streams are dropped. Surver uses recorded per-block
relation counts to serve fixed-size revisioned relation pages by decoding only
the overlapping chunks; transaction record pages are decoded unlinked and do
not force the global relation body resident.

## Konata paged-detail acceptance measurements

The ignored automated acceptance test
`multi_million_rows_keep_decoded_detail_bounded_under_forced_eviction` builds
the common encoded detail-page representation and then random-accesses cold
pages with the decoded cache constrained to one page. On 2026-07-10, a Linux
x86-64 dev/test build measured on an Intel Core Ultra 7 265K (20 cores), Linux
7.0.0-27-generic, with rustc/cargo 1.95.0:

| Rows | Stages | Pages (4,096 rows) | Encoded backing | Build | Forced decoded budget |
|---:|---:|---:|---:|---:|---:|
| 2,000,000 | 2,000,000 | 489 | 36 MiB | 1.10 s | 224 KiB (one page) |

The companion ignored test
`multi_million_projected_rows_keep_model_and_detail_queries_bounded` feeds
4,096-row synthetic transaction batches through the production compact record
projector and two million `parent_of` records through the incremental relation
join (dropping every generic batch), then constructs the complete row
directory, lookup/range/visibility indexes, two million stage records, and
encoded detail store. On the same 2026-07-10 dev/test environment it completed
in 5.24 s, produced 489 detail pages / 32 MiB encoded backing, resolved the
final transaction and event identities exactly, and kept cold decoded detail
to a 224 KiB one-page budget under forced eviction.

The test asserts that eviction remains active, resident decoded bytes stay
within the budget, and the compact backing remains below the architecture's
24-byte normal-stage target plus CSR row offsets. These figures cover the
detail store rather than end-to-end FTR parsing or frame latency; they are a
reproducible storage/eviction gate, not by themselves an end-to-end
multi-million-row or release-build performance claim.
