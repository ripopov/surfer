//! Support for the FTR transaction event convention.
//!
//! An *event* is a transaction written through a `<parent>.events` generator
//! and linked to exactly one parent transaction by an incoming `parent_of`
//! relation (parent = relation source). See `docs/development/FTR_EVENTS.md`
//! for the precise convention and `docs/development/FtrEventsSufer.md` for the
//! UX this module backs.
//!
//! [`EventIndex`] is rebuilt whenever a stream is loaded into memory and
//! answers, in O(1):
//! - is a generator an events generator, and for which parent generator?
//! - which parent transaction does an event belong to (plus violation flags)?
//! - which events does a parent transaction carry (sorted by time)?

use ftr_parser::types::{AttributeType, FTR, GeneratorId, StreamId, Transaction, TransactionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Generator name suffix that opts a generator into event semantics.
pub const EVENTS_GENERATOR_SUFFIX: &str = ".events";
/// Relation name linking a parent transaction (source) to an event (sink).
pub const EVENT_PARENT_RELATION: &str = "parent_of";
/// `BEGIN` attribute holding the event name.
pub const EVENT_NAME_ATTRIBUTE: &str = "name";

/// How events of a parent generator are presented on its displayed row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventDisplayMode {
    /// Markers drawn on top of the parent transaction bars (default).
    #[default]
    Overlay,
    /// Events drawn on dedicated lanes directly below the parent lanes.
    SeparateRow,
    /// Parent transactions only; events are not drawn.
    Hidden,
}

/// Per-event information resolved from the `parent_of` relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventInfo {
    /// The parent transaction (source of the first incoming `parent_of`).
    pub parent_tx: TransactionId,
    /// Generator of the parent transaction.
    pub parent_gen: GeneratorId,
    /// More than one incoming `parent_of` relation was recorded (malformed
    /// per the convention; the first parent is used).
    pub multiple_parents: bool,
    /// The event's time range lies (partly) outside the parent's range.
    pub out_of_range: bool,
}

/// Index over the event structure of a loaded FTR trace.
#[derive(Debug, Default)]
pub struct EventIndex {
    /// events generator -> parent generator
    events_to_parent_gen: HashMap<GeneratorId, GeneratorId>,
    /// parent generator -> events generator
    parent_to_events_gen: HashMap<GeneratorId, GeneratorId>,
    /// any loaded transaction -> (generator, index within the generator)
    tx_lookup: HashMap<TransactionId, (GeneratorId, usize)>,
    /// loaded transaction scoped by stream -> (generator, index within the generator)
    tx_lookup_by_stream: HashMap<(StreamId, TransactionId), (GeneratorId, usize)>,
    /// event transaction -> resolved parent info
    event_info: HashMap<TransactionId, EventInfo>,
    /// parent transaction -> events sorted by (start time, transaction id)
    parent_events: HashMap<TransactionId, Vec<TransactionId>>,
    /// generator -> number of display lanes (max transaction row + 1)
    gen_lanes: HashMap<GeneratorId, usize>,
    /// events generator -> number of conforming (non-orphan) events
    conforming_counts: HashMap<GeneratorId, usize>,
}

impl EventIndex {
    /// Builds the index from a (partially) loaded trace. Generator pairing
    /// only needs header data; per-transaction info covers loaded streams.
    #[must_use]
    pub fn build(ftr: &FTR) -> Self {
        let mut index = EventIndex::default();
        index.pair_generators(ftr);
        index.build_tx_lookup(ftr);
        index.resolve_events(ftr);
        index
    }

    /// Pairs `<base>.events` generators with their `<base>` sibling in the
    /// same stream. A `foo.events` generator without a `foo` sibling is an
    /// ordinary generator and gets no entry.
    fn pair_generators(&mut self, ftr: &FTR) {
        for stream in ftr.tx_streams.values() {
            let names: HashMap<&str, GeneratorId> = stream
                .generators
                .iter()
                .filter_map(|id| ftr.tx_generators.get(id))
                .map(|generator| (generator.name.as_str(), generator.id))
                .collect();

            let pairs = names.iter().filter_map(|(name, events_gen)| {
                name.strip_suffix(EVENTS_GENERATOR_SUFFIX)
                    .filter(|base| !base.is_empty())
                    .and_then(|base| names.get(base))
                    .map(|parent_gen| (*events_gen, *parent_gen))
            });

            for (events_gen, parent_gen) in pairs {
                self.events_to_parent_gen.insert(events_gen, parent_gen);
                self.parent_to_events_gen.insert(parent_gen, events_gen);
            }
        }
    }

    fn build_tx_lookup(&mut self, ftr: &FTR) {
        self.tx_lookup = ftr
            .tx_generators
            .values()
            .flat_map(|generator| {
                generator
                    .transactions
                    .iter()
                    .enumerate()
                    .map(|(idx, tx)| (tx.get_tx_id(), (generator.id, idx)))
            })
            .collect();
        self.tx_lookup_by_stream =
            ftr.tx_generators
                .values()
                .flat_map(|generator| {
                    generator.transactions.iter().enumerate().map(|(idx, tx)| {
                        ((generator.stream_id, tx.get_tx_id()), (generator.id, idx))
                    })
                })
                .collect();
        self.gen_lanes = ftr
            .tx_generators
            .values()
            .map(|generator| {
                let lanes = generator
                    .transactions
                    .iter()
                    .map(|tx| tx.row + 1)
                    .max()
                    .unwrap_or(1);
                (generator.id, lanes)
            })
            .collect();
    }

    /// Resolves each event's parent through its incoming `parent_of`
    /// relations. Events without a resolvable parent stay orphans (no entry).
    fn resolve_events(&mut self, ftr: &FTR) {
        for (events_gen, expected_parent_gen) in self.events_to_parent_gen.clone() {
            let Some(generator) = ftr.tx_generators.get(&events_gen) else {
                continue;
            };
            let mut conforming = 0;
            for tx in generator.transactions.iter() {
                if let Some(info) = self.resolve_event(ftr, tx, expected_parent_gen) {
                    self.parent_events
                        .entry(info.parent_tx)
                        .or_default()
                        .push(tx.get_tx_id());
                    self.event_info.insert(tx.get_tx_id(), info);
                    conforming += 1;
                }
            }
            self.conforming_counts.insert(events_gen, conforming);
        }

        // Sort each parent's events by time for stable presentation order
        for events in self.parent_events.values_mut() {
            events.sort_unstable_by_key(|tx_id| {
                let start_time = self
                    .tx_lookup
                    .get(tx_id)
                    .and_then(|(gen_id, idx)| ftr.tx_generators.get(gen_id)?.transactions.get(*idx))
                    .map(ftr_parser::types::Transaction::get_start_time);
                (start_time, tx_id.0)
            });
        }
    }

    fn resolve_event(
        &self,
        ftr: &FTR,
        tx: &Transaction,
        expected_parent_gen: GeneratorId,
    ) -> Option<EventInfo> {
        let parent_relations = tx
            .inc_relations
            .iter()
            .filter_map(|idx| ftr.get_relation(*idx))
            .filter(|rel| rel.name.as_ref() == EVENT_PARENT_RELATION)
            .collect::<Vec<_>>();

        let parent_relation = parent_relations.first()?;
        let parent_tx_id = parent_relation.source_tx_id;
        let (parent_gen, parent_idx) = self
            .tx_lookup_by_stream
            .get(&(parent_relation.source_stream_id, parent_tx_id))
            .or_else(|| self.tx_lookup.get(&parent_tx_id))
            .copied()?;
        if parent_gen != expected_parent_gen {
            return None;
        }
        let parent = ftr
            .tx_generators
            .get(&parent_gen)?
            .transactions
            .get(parent_idx)?;

        let out_of_range = tx.event.start_time < parent.event.start_time
            || tx.event.end_time > parent.event.end_time;

        Some(EventInfo {
            parent_tx: parent_tx_id,
            parent_gen,
            multiple_parents: parent_relations.len() > 1,
            out_of_range,
        })
    }

    /// The events generator paired with `parent_gen`, if any.
    #[must_use]
    pub fn events_generator_of(&self, parent_gen: GeneratorId) -> Option<GeneratorId> {
        self.parent_to_events_gen.get(&parent_gen).copied()
    }

    /// The paired events generator, but only once it has at least one
    /// conforming parent relation for `parent_gen`.
    #[must_use]
    pub fn conforming_events_generator_of(&self, parent_gen: GeneratorId) -> Option<GeneratorId> {
        self.events_generator_of(parent_gen)
            .filter(|events_gen| self.conforming_event_count(*events_gen) > 0)
    }

    /// The parent generator paired with `events_gen`, if any.
    #[must_use]
    pub fn parent_generator_of(&self, events_gen: GeneratorId) -> Option<GeneratorId> {
        self.events_to_parent_gen.get(&events_gen).copied()
    }

    /// Whether `gen_id` is a conforming events generator.
    #[must_use]
    pub fn is_events_generator(&self, gen_id: GeneratorId) -> bool {
        self.events_to_parent_gen.contains_key(&gen_id)
    }

    /// Whether `gen_id` is a paired events generator with at least one
    /// conforming event. Orphan-only `.events` generators stay ordinary.
    #[must_use]
    pub fn is_conforming_events_generator(&self, gen_id: GeneratorId) -> bool {
        self.is_events_generator(gen_id) && self.conforming_event_count(gen_id) > 0
    }

    /// Parent info for an event transaction; `None` for non-events and
    /// orphan events.
    #[must_use]
    pub fn event_info(&self, tx_id: TransactionId) -> Option<&EventInfo> {
        self.event_info.get(&tx_id)
    }

    /// Events of a parent transaction, sorted by (start time, id).
    #[must_use]
    pub fn events_of_parent(&self, tx_id: TransactionId) -> &[TransactionId] {
        self.parent_events.get(&tx_id).map_or(&[], Vec::as_slice)
    }

    /// Location of any loaded transaction as (generator, index in generator).
    #[must_use]
    pub fn lookup_tx(&self, tx_id: TransactionId) -> Option<(GeneratorId, usize)> {
        self.tx_lookup.get(&tx_id).copied()
    }

    /// Number of display lanes a generator's transactions occupy
    /// (max transaction row + 1; 1 for empty/unloaded generators).
    #[must_use]
    pub fn lane_count(&self, gen_id: GeneratorId) -> usize {
        self.gen_lanes.get(&gen_id).copied().unwrap_or(1)
    }

    /// Number of conforming (non-orphan) events in an events generator.
    /// Zero when the stream is not loaded yet.
    #[must_use]
    pub fn conforming_event_count(&self, events_gen: GeneratorId) -> usize {
        self.conforming_counts
            .get(&events_gen)
            .copied()
            .unwrap_or(0)
    }
}

/// Whether a transaction in `generator` is an orphan event: it lives in a
/// conforming events generator but has no resolvable `parent_of` parent.
#[must_use]
pub fn is_orphan_event(index: &EventIndex, generator: GeneratorId, tx_id: TransactionId) -> bool {
    index.is_events_generator(generator) && index.event_info(tx_id).is_none()
}

/// The event name from the `BEGIN` attribute called `name` (rule 6 of the
/// convention), falling back to any attribute called `name` for graceful
/// degradation on non-conforming writers.
#[must_use]
pub fn event_name(tx: &Transaction) -> Option<String> {
    tx.attributes
        .iter()
        .find(|attr| {
            matches!(attr.kind, AttributeType::BEGIN) && attr.name.as_ref() == EVENT_NAME_ATTRIBUTE
        })
        .or_else(|| {
            tx.attributes
                .iter()
                .find(|attr| attr.name.as_ref() == EVENT_NAME_ATTRIBUTE)
        })
        .map(ftr_parser::types::Attribute::value)
}

/// `BEGIN` attributes of a transaction, used for the parent summary in the
/// event details card.
#[must_use]
pub fn begin_attributes(tx: &Transaction) -> Vec<&ftr_parser::types::Attribute> {
    tx.attributes
        .iter()
        .filter(|attr| matches!(attr.kind, AttributeType::BEGIN))
        .collect()
}

/// Builders that construct `FTR` traces through serde, so tests can model
/// edge cases (orphans, multiple parents, out-of-range events) without
/// binary fixture files: several ftr_parser fields are not publicly
/// constructible.
#[cfg(test)]
pub(crate) mod test_util {
    use ftr_parser::types::FTR;
    use serde_json::json;

    pub fn ftr_from_json(streams: serde_json::Value, generators: serde_json::Value) -> FTR {
        // Transactions are written with their incoming relations inline (see
        // `tx`), but the parser stores relations once in `FTR::tx_relations`
        // and only indices on the transactions. Hoist the inline objects into
        // the shared list and replace them with their indices.
        let mut generators = generators;
        let mut relations: Vec<serde_json::Value> = vec![];
        if let Some(generators) = generators.as_object_mut() {
            for transaction in generators
                .values_mut()
                .filter_map(|g| g.get_mut("transactions")?.as_array_mut())
                .flatten()
            {
                if let Some(inline) = transaction
                    .get_mut("inc_relations")
                    .and_then(|r| r.as_array_mut())
                {
                    let indices: Vec<serde_json::Value> = inline
                        .drain(..)
                        .map(|rel| {
                            relations.push(rel);
                            json!(relations.len() - 1)
                        })
                        .collect();
                    inline.extend(indices);
                }
            }
        }
        serde_json::from_value(json!({
            "time_scale": "Ns",
            "max_timestamp": 1000,
            "str_dict": {},
            "tx_streams": streams,
            "tx_generators": generators,
            "tx_relations": relations,
            "path": null,
        }))
        .expect("constructing test FTR")
    }

    pub fn stream(id: usize, name: &str, generators: &[usize]) -> serde_json::Value {
        json!({
            "id": id,
            "name": name,
            "kind": "transactions",
            "generators": generators,
            "transactions_loaded": true,
            "tx_block_ids": [],
        })
    }

    pub fn generator(
        id: usize,
        stream_id: usize,
        name: &str,
        transactions: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "id": id,
            "stream_id": stream_id,
            "name": name,
            "transactions": transactions,
        })
    }

    /// A transaction on display `row` 0. `name` becomes the `BEGIN` `name`
    /// attribute; `inc_parents` are incoming `parent_of` relations given as
    /// (source transaction, source stream) pairs.
    pub fn tx(
        id: usize,
        gen_id: usize,
        start: u64,
        end: u64,
        name: Option<&str>,
        inc_parents: &[(usize, usize)],
    ) -> serde_json::Value {
        tx_on_row(id, gen_id, start, end, name, inc_parents, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tx_on_row(
        id: usize,
        gen_id: usize,
        start: u64,
        end: u64,
        name: Option<&str>,
        inc_parents: &[(usize, usize)],
        row: usize,
    ) -> serde_json::Value {
        let attributes: Vec<serde_json::Value> = name
            .map(|n| {
                json!({
                    "kind": "BEGIN",
                    "name": "name",
                    "data_type": {"String": n},
                })
            })
            .into_iter()
            .collect();
        let inc_relations: Vec<serde_json::Value> = inc_parents
            .iter()
            .map(|(src_tx, src_stream)| {
                json!({
                    "name": "parent_of",
                    "source_tx_id": src_tx,
                    "sink_tx_id": id,
                    "source_stream_id": src_stream,
                    "sink_stream_id": src_stream,
                })
            })
            .collect();
        json!({
            "event": {
                "tx_id": id,
                "gen_id": gen_id,
                "start_time": start,
                "end_time": end,
            },
            "attributes": attributes,
            "inc_relations": inc_relations,
            "out_relations": [],
            "row": row,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::{ftr_from_json, generator, stream, tx};
    use super::*;
    use ftr_parser::types::FTR;
    use serde_json::json;

    /// One stream: parent generator 10 ("work") with events generator 11
    /// ("work.events"). Parent tx 1 spans [100, 200].
    fn simple_trace(events: serde_json::Value) -> FTR {
        ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11])}),
            json!({
                "10": generator(10, 1, "work", json!([tx(1, 10, 100, 200, None, &[])])),
                "11": generator(11, 1, "work.events", events),
            }),
        )
    }

    #[test]
    fn pairs_events_generator_with_sibling() {
        let ftr = simple_trace(json!([]));
        let index = EventIndex::build(&ftr);
        assert_eq!(
            index.events_generator_of(GeneratorId(10)),
            Some(GeneratorId(11))
        );
        assert_eq!(
            index.parent_generator_of(GeneratorId(11)),
            Some(GeneratorId(10))
        );
        assert!(index.is_events_generator(GeneratorId(11)));
        assert!(!index.is_events_generator(GeneratorId(10)));
    }

    #[test]
    fn events_generator_without_sibling_is_ordinary() {
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10])}),
            json!({"10": generator(10, 1, "lonely.events", json!([]))}),
        );
        let index = EventIndex::build(&ftr);
        assert!(!index.is_events_generator(GeneratorId(10)));
        assert_eq!(index.parent_generator_of(GeneratorId(10)), None);
    }

    #[test]
    fn sibling_in_other_stream_does_not_pair() {
        let ftr = ftr_from_json(
            json!({
                "1": stream(1, "a", &[10]),
                "2": stream(2, "b", &[11]),
            }),
            json!({
                "10": generator(10, 1, "work", json!([])),
                "11": generator(11, 2, "work.events", json!([])),
            }),
        );
        let index = EventIndex::build(&ftr);
        assert!(!index.is_events_generator(GeneratorId(11)));
    }

    #[test]
    fn bare_dot_events_name_does_not_pair() {
        // ".events" has an empty base name and never pairs
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11])}),
            json!({
                "10": generator(10, 1, "", json!([])),
                "11": generator(11, 1, ".events", json!([])),
            }),
        );
        let index = EventIndex::build(&ftr);
        assert!(!index.is_events_generator(GeneratorId(11)));
    }

    #[test]
    fn resolves_event_parent() {
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, Some("stall"), &[(1, 1)])]));
        let index = EventIndex::build(&ftr);

        let info = index.event_info(TransactionId(2)).expect("event resolved");
        assert_eq!(info.parent_tx, TransactionId(1));
        assert_eq!(info.parent_gen, GeneratorId(10));
        assert!(!info.multiple_parents);
        assert!(!info.out_of_range);
        assert_eq!(
            index.events_of_parent(TransactionId(1)),
            &[TransactionId(2)]
        );
    }

    #[test]
    fn orphan_event_has_no_info() {
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, Some("stall"), &[])]));
        let index = EventIndex::build(&ftr);
        assert_eq!(index.event_info(TransactionId(2)), None);
        assert!(is_orphan_event(&index, GeneratorId(11), TransactionId(2)));
        assert!(index.events_of_parent(TransactionId(1)).is_empty());
    }

    #[test]
    fn orphan_only_paired_generator_is_not_conforming() {
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, Some("stall"), &[])]));
        let index = EventIndex::build(&ftr);

        assert_eq!(
            index.events_generator_of(GeneratorId(10)),
            Some(GeneratorId(11))
        );
        assert_eq!(index.conforming_events_generator_of(GeneratorId(10)), None);
        assert!(!index.is_conforming_events_generator(GeneratorId(11)));
        assert_eq!(index.conforming_event_count(GeneratorId(11)), 0);
    }

    #[test]
    fn unresolvable_parent_is_orphan() {
        // parent_of points at a transaction that does not exist
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, Some("stall"), &[(99, 1)])]));
        let index = EventIndex::build(&ftr);
        assert_eq!(index.event_info(TransactionId(2)), None);
        assert!(is_orphan_event(&index, GeneratorId(11), TransactionId(2)));
    }

    #[test]
    fn multiple_parents_flagged_first_used() {
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11])}),
            json!({
                "10": generator(10, 1, "work", json!([
                    tx(1, 10, 100, 200, None, &[]),
                    tx(3, 10, 100, 200, None, &[]),
                ])),
                "11": generator(11, 1, "work.events", json!([
                    tx(2, 11, 150, 150, Some("stall"), &[(1, 1), (3, 1)]),
                ])),
            }),
        );
        let index = EventIndex::build(&ftr);
        let info = index.event_info(TransactionId(2)).expect("event resolved");
        assert_eq!(info.parent_tx, TransactionId(1));
        assert!(info.multiple_parents);
        assert_eq!(
            index.events_of_parent(TransactionId(1)),
            &[TransactionId(2)]
        );
        assert!(index.events_of_parent(TransactionId(3)).is_empty());
    }

    #[test]
    fn out_of_range_event_flagged() {
        let ftr = simple_trace(json!([
            tx(2, 11, 50, 50, Some("early"), &[(1, 1)]),
            tx(3, 11, 150, 250, Some("overhang"), &[(1, 1)]),
            tx(4, 11, 100, 200, Some("exact"), &[(1, 1)]),
        ]));
        let index = EventIndex::build(&ftr);
        assert!(index.event_info(TransactionId(2)).unwrap().out_of_range);
        assert!(index.event_info(TransactionId(3)).unwrap().out_of_range);
        assert!(!index.event_info(TransactionId(4)).unwrap().out_of_range);
    }

    #[test]
    fn events_sorted_by_time_then_id() {
        let ftr = simple_trace(json!([
            tx(4, 11, 180, 180, Some("c"), &[(1, 1)]),
            tx(3, 11, 120, 120, Some("a"), &[(1, 1)]),
            tx(2, 11, 120, 120, Some("b"), &[(1, 1)]),
        ]));
        let index = EventIndex::build(&ftr);
        assert_eq!(
            index.events_of_parent(TransactionId(1)),
            &[TransactionId(2), TransactionId(3), TransactionId(4)]
        );
    }

    #[test]
    fn parent_of_to_non_events_generator_is_not_an_event() {
        // Ordinary hierarchy relation between two regular generators
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 12])}),
            json!({
                "10": generator(10, 1, "work", json!([tx(1, 10, 100, 200, None, &[])])),
                "12": generator(12, 1, "sub", json!([
                    tx(2, 12, 120, 180, None, &[(1, 1)]),
                ])),
            }),
        );
        let index = EventIndex::build(&ftr);
        assert_eq!(index.event_info(TransactionId(2)), None);
        assert!(index.events_of_parent(TransactionId(1)).is_empty());
        assert!(!index.is_events_generator(GeneratorId(12)));
    }

    #[test]
    fn parent_must_belong_to_paired_parent_generator() {
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11, 12])}),
            json!({
                "10": generator(10, 1, "work", json!([tx(1, 10, 100, 200, None, &[])])),
                "11": generator(11, 1, "work.events", json!([
                    tx(2, 11, 150, 150, Some("wrong_parent"), &[(9, 1)]),
                ])),
                "12": generator(12, 1, "other", json!([tx(9, 12, 100, 200, None, &[])])),
            }),
        );
        let index = EventIndex::build(&ftr);

        assert_eq!(index.event_info(TransactionId(2)), None);
        assert!(is_orphan_event(&index, GeneratorId(11), TransactionId(2)));
        assert_eq!(index.conforming_events_generator_of(GeneratorId(10)), None);
        assert!(index.events_of_parent(TransactionId(9)).is_empty());
    }

    #[test]
    fn event_name_prefers_begin_attribute() {
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, Some("stall"), &[(1, 1)])]));
        let generator = ftr.get_generator(GeneratorId(11)).unwrap();
        assert_eq!(
            event_name(&generator.transactions[0]),
            Some("stall".to_string())
        );
    }

    #[test]
    fn event_name_missing_is_none() {
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, None, &[(1, 1)])]));
        let generator = ftr.get_generator(GeneratorId(11)).unwrap();
        assert_eq!(event_name(&generator.transactions[0]), None);
    }

    #[test]
    fn conforming_event_count_skips_orphans() {
        let ftr = simple_trace(json!([
            tx(2, 11, 150, 150, Some("a"), &[(1, 1)]),
            tx(3, 11, 160, 160, Some("b"), &[]),
        ]));
        let index = EventIndex::build(&ftr);
        assert_eq!(index.conforming_event_count(GeneratorId(11)), 1);
    }

    #[test]
    fn lookup_tx_finds_loaded_transactions() {
        let ftr = simple_trace(json!([tx(2, 11, 150, 150, Some("stall"), &[(1, 1)])]));
        let index = EventIndex::build(&ftr);
        assert_eq!(
            index.lookup_tx(TransactionId(1)),
            Some((GeneratorId(10), 0))
        );
        assert_eq!(
            index.lookup_tx(TransactionId(2)),
            Some((GeneratorId(11), 0))
        );
        assert_eq!(index.lookup_tx(TransactionId(99)), None);
    }
}
