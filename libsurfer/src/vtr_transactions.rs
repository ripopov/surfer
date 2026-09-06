//! VTR transaction adapter.
//!
//! The transaction widgets currently consume `ftr_parser`'s in-memory model.
//! This adapter preserves VTR's stream/generator grouping, attributes and
//! relations at that boundary while the native VTR reader remains read-only.

use crate::transaction_index::{Span, TrackIndex, TrackKey};
use std::sync::Arc;

use crate::transaction_container::{
    TransactionContainer, VtrTransactionDetails, VtrTransactionEvent, VtrTransactionRelation,
    VtrTransactionStage,
};
use ftr_parser::types::{
    Attribute, AttributeType, DataType, Event, FTR, GeneratorId, StreamId, Timescale,
    Transaction as FtrTransaction, TransactionId, TxGenerator, TxRelation, TxStream,
};
use num::BigInt;
use std::collections::HashMap;
use vtr::{NodeData, Reader, TxQuery, Value};

/// Canonical generator/window requests from visible waveform tiles, plus the
/// transaction selected in a visible inspector. Empty demand owns no payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TransactionDemand {
    pub windows: Vec<(u32, u64, u64)>,
    pub pinned: Vec<u64>,
    pub payloads: Vec<u64>,
}

impl TransactionDemand {
    pub fn normalize(&mut self) {
        // Track geometry is independent of time; payload demand comes from screen samples.
        for (_, start, end) in &mut self.windows {
            *start = 0;
            *end = u64::MAX;
        }
        self.payloads.sort_unstable();
        self.payloads.dedup();
        self.windows.sort_unstable();
        let mut merged: Vec<(u32, u64, u64)> = Vec::new();
        for (generator, start, end) in self.windows.drain(..) {
            if let Some(last) = merged.last_mut()
                && last.0 == generator
                && start <= last.2.saturating_add(1)
            {
                last.2 = last.2.max(end);
            } else {
                merged.push((generator, start, end));
            }
        }
        self.windows = merged;
        self.pinned.sort_unstable();
        self.pinned.dedup();
    }
}

pub(crate) struct NativeTransactions {
    pub tracks: HashMap<TrackKey, Arc<TrackIndex>>,
    pub logs: std::sync::Arc<crate::tile_kinds::simulation_logs::native::Source>,
    reader: std::sync::Arc<std::sync::Mutex<Reader>>,
    identity: u64,
    pub desired: TransactionDemand,
    pub loaded: TransactionDemand,
    pub in_flight: bool,
}

impl NativeTransactions {
    pub fn new(reader: std::sync::Arc<std::sync::Mutex<Reader>>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self {
            tracks: HashMap::new(),
            logs: crate::tile_kinds::simulation_logs::native::Source::new(reader.clone()),
            reader,
            identity: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            desired: Default::default(),
            loaded: Default::default(),
            in_flight: false,
        }
    }
}

pub(crate) struct TransactionLoad {
    tracks: HashMap<TrackKey, Arc<TrackIndex>>,
    reader: std::sync::Arc<std::sync::Mutex<Reader>>,
    identity: u64,
    demand: TransactionDemand,
}

pub struct TransactionLoadResult {
    tracks: HashMap<TrackKey, Arc<TrackIndex>>,
    identity: u64,
    demand: TransactionDemand,
    data: Result<Option<TransactionContainer>, String>,
}

impl TransactionLoad {
    pub fn run(mut self) -> TransactionLoadResult {
        let mut reader = self.reader.lock().unwrap();
        // Geometry belongs to complete tracks, not viewport payload batches.
        // Reuse these immutable indexes when windows change or a result is stale.
        let data = (|| {
            for &(generator, _, _) in &self.demand.windows {
                let Some(stream) = reader.generator_stream(vtr::NodeId(generator)) else {
                    continue;
                };
                let key = TrackKey {
                    stream: stream.0,
                    generator: None,
                };
                if self.tracks.contains_key(&key) {
                    continue;
                }
                let mut spans = Vec::new();
                reader
                    .visit_transactions(
                        &TxQuery {
                            stream: Some(stream),
                            ..Default::default()
                        },
                        |tx| {
                            spans.push(Span {
                                id: tx.id,
                                begin: tx.begin,
                                end: tx.end,
                                generator: tx.generator.0,
                            });
                            true
                        },
                    )
                    .map_err(|e| e.to_string())?;
                let mut generators: HashMap<u32, Vec<Span>> = HashMap::new();
                for span in &spans {
                    generators.entry(span.generator).or_default().push(*span);
                }
                self.tracks.insert(key, Arc::new(TrackIndex::new(spans)));
                for (generator, spans) in generators {
                    self.tracks.insert(
                        TrackKey {
                            stream: stream.0,
                            generator: Some(generator),
                        },
                        Arc::new(TrackIndex::new(spans)),
                    );
                }
                reader.clear_cache();
            }
            let mut ids = self.demand.payloads.clone();
            ids.extend(self.demand.pinned.iter().copied());
            ids.sort_unstable();
            ids.dedup();
            from_reader(&reader, &[], &ids)
        })();
        reader.clear_cache();
        TransactionLoadResult {
            tracks: self.tracks,
            identity: self.identity,
            demand: self.demand,
            data,
        }
    }
}

impl TransactionContainer {
    pub(crate) fn retain_native_transactions(
        &mut self,
        mut demand: TransactionDemand,
    ) -> Option<TransactionLoad> {
        let native = self.native.as_mut()?;
        demand.normalize();
        native.desired = demand;
        if native.desired.windows.is_empty()
            && native.desired.pinned.is_empty()
            && native.desired.payloads.is_empty()
        {
            for generator in self.inner.tx_generators.values_mut() {
                generator.transactions.clear();
            }
            if let Some(details) = &mut self.vtr_details {
                details.clear();
            }
            self.locations.get_mut().unwrap().clear();
            native.loaded = native.desired.clone();
            return None;
        }
        if native.in_flight || native.loaded == native.desired {
            return None;
        }
        native.in_flight = true;
        Some(TransactionLoad {
            tracks: native.tracks.clone(),
            reader: native.reader.clone(),
            identity: native.identity,
            demand: native.desired.clone(),
        })
    }

    pub(crate) fn on_native_transactions_loaded(&mut self, result: TransactionLoadResult) -> bool {
        let Some(native) = &mut self.native else {
            return false;
        };
        if native.identity != result.identity {
            return false;
        }
        native.in_flight = false;
        native.tracks.extend(result.tracks);
        if native.desired != result.demand {
            return false;
        }
        // A failed demand is not retried in a tight loop; changing demand permits a retry.
        native.loaded = result.demand;
        match result.data {
            Ok(Some(data)) => {
                self.inner = data.inner;
                self.locations = data.locations;
                self.vtr_details = data.vtr_details;
                true
            }
            Ok(None) => false,
            Err(error) => {
                tracing::error!("Failed to load VTR transactions: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
fn to_ftr(path: &std::path::Path) -> Result<Option<TransactionContainer>, String> {
    let reader = Reader::open(path).map_err(|error| error.to_string())?;
    from_reader(&reader, &[TxQuery::default()], &[])
}

pub(crate) fn from_reader(
    reader: &Reader,
    queries: &[TxQuery],
    pinned: &[u64],
) -> Result<Option<TransactionContainer>, String> {
    if reader.tx_counts().0 == 0 {
        return Ok(None);
    }

    let mut streams = HashMap::new();
    for node in reader.streams() {
        let NodeData::Stream { kind } = reader.hierarchy().node(node).data else {
            continue;
        };
        let stream: TxStream = serde_json::from_value(serde_json::json!({
            "id": node.0,
            "name": reader.name(node),
            "kind": reader.str(kind),
            "generators": [],
            "transactions_loaded": true,
            "tx_block_ids": []
        }))
        .map_err(|error| error.to_string())?;
        streams.insert(StreamId(node.0 as usize), stream);
    }

    let mut generators = HashMap::new();
    let mut generator_to_stream = HashMap::new();
    let mut vtr_details = HashMap::new();
    for node in reader.generators() {
        let Some(stream) = reader.generator_stream(node) else {
            continue;
        };
        let stream_id = StreamId(stream.0 as usize);
        if !streams.contains_key(&stream_id) {
            continue;
        }
        let generator_id = GeneratorId(node.0 as usize);
        generator_to_stream.insert(generator_id, stream_id);
        streams
            .get_mut(&stream_id)
            .unwrap()
            .generators
            .push(generator_id);
        generators.insert(
            generator_id,
            TxGenerator {
                id: generator_id,
                stream_id,
                name: reader.name(node).to_owned(),
                transactions: Vec::new(),
            },
        );
    }

    let mut selected = std::collections::BTreeMap::new();
    for query in queries {
        reader
            .visit_transactions(query, |tx| {
                selected.entry(tx.id).or_insert_with(|| tx.clone());
                true
            })
            .map_err(|error| error.to_string())?;
    }
    for id in pinned {
        if let Some(tx) = reader.transaction(*id).map_err(|e| e.to_string())? {
            selected.insert(tx.id, tx);
        }
    }
    for tx in selected.into_values() {
        let Some(generator) = generators.get_mut(&GeneratorId(tx.generator.0 as usize)) else {
            continue;
        };
        let tx_id = TransactionId(tx.id as usize);
        vtr_details.insert(
            tx_id,
            VtrTransactionDetails {
                status: tx.status.name().to_owned(),
                kind: tx_kind_name(tx.kind).to_owned(),
                parent: tx.parent.map(|id| TransactionId(id as usize)),
                events: tx
                    .events
                    .iter()
                    .map(|event| VtrTransactionEvent {
                        time: event.time,
                        name: reader.str(event.name).to_owned(),
                        attrs: to_named_attrs(reader, &event.attrs),
                    })
                    .collect(),
                stages: tx
                    .stages
                    .iter()
                    .map(|stage| VtrTransactionStage {
                        name: reader.str(stage.name).to_owned(),
                        lane: reader.str(stage.lane).to_owned(),
                        begin: stage.begin,
                        end: stage.end,
                        attrs: to_named_attrs(reader, &stage.attrs),
                    })
                    .collect(),
                relations: Vec::new(),
            },
        );
        generator.transactions.push(FtrTransaction {
            event: Event {
                tx_id,
                gen_id: generator.id,
                start_time: tx.begin.into(),
                end_time: tx.end.into(),
            },
            attributes: tx
                .attrs
                .iter()
                .map(|attr| Attribute {
                    kind: match attr.phase {
                        vtr::AttrPhase::Begin => AttributeType::BEGIN,
                        vtr::AttrPhase::End => AttributeType::END,
                        vtr::AttrPhase::Record => AttributeType::RECORD,
                    },
                    name: reader.str(attr.key).to_owned(),
                    data_type: to_ftr_value(reader, &attr.value),
                })
                .collect(),
            inc_relations: Vec::new(),
            out_relations: Vec::new(),
            row: 0,
        });
    }

    // Transaction IDs and file order need not follow simulation time.
    for generator in generators.values_mut() {
        generator
            .transactions
            .sort_by_key(|tx| (tx.get_start_time(), tx.get_tx_id().0));
    }

    let mut tx_locations = HashMap::new();
    for (generator_id, generator) in &generators {
        for (index, tx) in generator.transactions.iter().enumerate() {
            tx_locations.insert(tx.get_tx_id(), (*generator_id, index));
        }
    }
    let mut relations = Vec::new();
    for id in tx_locations.keys() {
        relations.extend(
            reader
                .relations_from(id.0 as u64)
                .map_err(|e| e.to_string())?,
        );
        relations.extend(
            reader
                .relations_to(id.0 as u64)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|relation| {
                    !tx_locations.contains_key(&TransactionId(relation.from as usize))
                }),
        );
    }
    let mut endpoint_streams = HashMap::new();
    for relation in &relations {
        for id in [relation.from, relation.to] {
            if endpoint_streams.contains_key(&id) {
                continue;
            }
            let generator =
                if let Some((generator, _)) = tx_locations.get(&TransactionId(id as usize)) {
                    Some(*generator)
                } else {
                    reader
                        .transaction(id)
                        .map_err(|e| e.to_string())?
                        .map(|tx| GeneratorId(tx.generator.0 as usize))
                };
            if let Some(stream) = generator.and_then(|g| generator_to_stream.get(&g)) {
                endpoint_streams.insert(id, *stream);
            }
        }
    }
    relations.iter().for_each(|relation| {
        let from = tx_locations
            .get(&TransactionId(relation.from as usize))
            .copied();
        let to = tx_locations
            .get(&TransactionId(relation.to as usize))
            .copied();
        let Some(from_stream) = endpoint_streams.get(&relation.from).copied() else {
            return;
        };
        let Some(to_stream) = endpoint_streams.get(&relation.to).copied() else {
            return;
        };
        let relation_name = reader.str(relation.kind).to_owned();
        let relation_attrs = to_named_attrs(reader, &relation.attrs);
        let outgoing = TxRelation {
            name: relation_name.clone(),
            source_tx_id: TransactionId(relation.from as usize),
            sink_tx_id: TransactionId(relation.to as usize),
            source_stream_id: from_stream,
            sink_stream_id: to_stream,
        };
        let incoming = TxRelation {
            name: relation_name.clone(),
            source_tx_id: TransactionId(relation.from as usize),
            sink_tx_id: TransactionId(relation.to as usize),
            source_stream_id: from_stream,
            sink_stream_id: to_stream,
        };
        if let Some((from_generator, from_index)) = from
            && let Some(tx) = generators
                .get_mut(&from_generator)
                .and_then(|generator| generator.transactions.get_mut(from_index))
        {
            tx.out_relations.push(outgoing);
        }
        if let Some((to_generator, to_index)) = to
            && let Some(tx) = generators
                .get_mut(&to_generator)
                .and_then(|generator| generator.transactions.get_mut(to_index))
        {
            tx.inc_relations.push(incoming);
        }
        let relation = VtrTransactionRelation {
            name: relation_name,
            source: TransactionId(relation.from as usize),
            target: TransactionId(relation.to as usize),
            attrs: relation_attrs,
        };
        if let Some(details) = vtr_details.get_mut(&relation.source) {
            details.relations.push(relation.clone());
        }
        if let Some(details) = vtr_details.get_mut(&relation.target) {
            details.relations.push(relation);
        }
    });

    let max_timestamp = reader
        .time_range()
        .map_or_else(|| BigInt::from(0), |(_, end)| BigInt::from(end));
    let mut ftr = FTR::default();
    ftr.time_scale = to_timescale(reader.meta().timescale);
    ftr.max_timestamp = max_timestamp;
    ftr.tx_streams = streams;
    ftr.tx_generators = generators;
    Ok(Some(TransactionContainer {
        locations: std::sync::Mutex::new(tx_locations),
        indexes: Default::default(),
        inner: ftr,
        vtr_details: Some(vtr_details),
        native: None,
    }))
}

fn tx_kind_name(kind: vtr::TxKind) -> &'static str {
    match kind {
        vtr::TxKind::Unspecified => "unspecified",
        vtr::TxKind::Internal => "internal",
        vtr::TxKind::Server => "server",
        vtr::TxKind::Client => "client",
        vtr::TxKind::Producer => "producer",
        vtr::TxKind::Consumer => "consumer",
    }
}

fn to_named_attrs(reader: &Reader, attrs: &[(vtr::StrId, Value)]) -> Vec<(String, String)> {
    attrs
        .iter()
        .map(|(key, value)| {
            let data_type = to_ftr_value(reader, value);
            let value = Attribute {
                kind: AttributeType::RECORD,
                name: String::new(),
                data_type,
            };
            (reader.str(*key).to_owned(), value.value())
        })
        .collect()
}

fn to_timescale(exponent: i8) -> Timescale {
    match exponent {
        -15 => Timescale::Fs,
        -12 => Timescale::Ps,
        -9 => Timescale::Ns,
        -6 => Timescale::Us,
        -3 => Timescale::Ms,
        0 => Timescale::S,
        _ => Timescale::None,
    }
}

fn to_ftr_value(reader: &Reader, value: &Value) -> DataType {
    match value {
        Value::Null => DataType::Error,
        Value::Bool(value) => DataType::Boolean(*value),
        Value::I64(value) => DataType::Integer(*value),
        Value::U64(value) => DataType::Unsigned(*value),
        Value::F64(value) => DataType::FloatingPointNumber(*value as f32),
        Value::Str(value) => DataType::String(reader.str(*value).to_owned()),
        Value::Text(value) => DataType::String(value.clone()),
        Value::Bytes(value) => DataType::String(
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        ),
        Value::Bits { width, data } => DataType::BitVector(unpack(data, *width, 2)),
        Value::Logic { width, data } => DataType::LogicVector(unpack(data, *width, 4)),
        Value::Logic9 { width, data } => DataType::LogicVector(unpack(data, *width, 9)),
        Value::Time(value) => DataType::Time(*value),
        Value::Enum { name, .. } => DataType::Enumeration(reader.str(*name).to_owned()),
        Value::Pointer(value) => DataType::Pointer(*value),
        Value::Fixed { raw, scale } => DataType::FixedPointInteger(*raw as f32 / 2f32.powi(*scale)),
        Value::UFixed { raw, scale } => {
            DataType::UnsignedFixedPointInteger(*raw as f32 / 2f32.powi(*scale))
        }
        Value::List(_) | Value::Map(_) => DataType::String(format!("{value:?}")),
    }
}

fn unpack(data: &[u8], width: u32, states: u8) -> String {
    let mut out = Vec::new();
    vtr::signal::unpack_ascii(data, width, states, &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_transactions_load_only_demand_and_reject_stale_results() {
        let loaded =
            crate::vtr_adapter::load(camino::Utf8Path::new("../examples/transactions.vtr"))
                .unwrap();
        let mut container = loaded.transactions.unwrap();
        assert!(
            container
                .inner
                .tx_generators
                .values()
                .all(|g| g.transactions.is_empty())
        );
        assert!(container.vtr_details.as_ref().unwrap().is_empty());
        let generator = container
            .get_generator_from_name(None, "issue".into())
            .unwrap()
            .id;
        let demand = TransactionDemand {
            windows: vec![(generator.0 as u32, 2, 2)],
            pinned: vec![],
            payloads: vec![1],
        };
        let load = container
            .retain_native_transactions(demand.clone())
            .unwrap();
        assert!(!container.is_fully_loaded());
        assert!(container.on_native_transactions_loaded(load.run()));
        let rows = &container.get_generator(generator).unwrap().transactions;
        assert_eq!(rows.len(), 1);
        let first = rows[0].get_tx_id();
        assert!(
            !container.vtr_details(first).unwrap().relations.is_empty(),
            "relations survive even when their other endpoint is unloaded"
        );
        assert!(container.retain_native_transactions(demand).is_none());
        let pin = TransactionDemand {
            windows: vec![],
            pinned: vec![first.0 as u64],
            payloads: vec![],
        };
        let load = container.retain_native_transactions(pin).unwrap();
        assert!(container.on_native_transactions_loaded(load.run()));
        assert_eq!(
            container
                .get_generator(generator)
                .unwrap()
                .transactions
                .len(),
            1
        );
        let load = container
            .retain_native_transactions(TransactionDemand {
                windows: vec![(generator.0 as u32, 0, 100)],
                pinned: vec![],
                payloads: vec![],
            })
            .unwrap();
        container.retain_native_transactions(Default::default());
        assert!(
            container
                .inner
                .tx_generators
                .values()
                .all(|g| g.transactions.is_empty())
        );
        assert!(container.vtr_details.as_ref().unwrap().is_empty());
        assert!(!container.on_native_transactions_loaded(load.run()));
        assert!(container.is_fully_loaded());
        assert!(
            container
                .inner
                .tx_generators
                .values()
                .all(|g| g.transactions.is_empty())
        );
    }

    #[test]
    fn complete_track_geometry_is_reused_and_payloads_are_screen_selected() {
        let loaded =
            crate::vtr_adapter::load(camino::Utf8Path::new("../examples/chi_noc.vtr")).unwrap();
        let mut container = loaded.transactions.unwrap();
        let generator = container
            .inner
            .tx_generators
            .values()
            .find(|g| g.name == "ReadShared")
            .unwrap();
        let key = TrackKey {
            stream: generator.stream_id.0 as u32,
            generator: None,
        };
        let generator_id = generator.id.0 as u32;
        let demand = TransactionDemand {
            windows: vec![(generator_id, 0, 260)],
            ..Default::default()
        };
        let load = container
            .retain_native_transactions(demand.clone())
            .unwrap();
        assert!(container.on_native_transactions_loaded(load.run()));
        assert!(
            container.vtr_details.as_ref().unwrap().is_empty(),
            "indexing does not materialize rich viewer payloads"
        );
        let index = container.native.as_ref().unwrap().tracks[&key].clone();
        assert_eq!(index.row_count(), 64);
        let samples = index.query(0..4, 400, 600, 1000);
        assert_eq!(samples.len(), 4);
        let payloads: Vec<_> = samples.iter().map(|s| s.representative.id).collect();
        let load = container
            .retain_native_transactions(TransactionDemand {
                payloads: payloads.clone(),
                ..demand.clone()
            })
            .unwrap();
        assert!(container.on_native_transactions_loaded(load.run()));
        assert_eq!(container.vtr_details.as_ref().unwrap().len(), 4);
        assert!(Arc::ptr_eq(
            &index,
            &container.native.as_ref().unwrap().tracks[&key]
        ));
        assert!(
            container
                .retain_native_transactions(TransactionDemand {
                    windows: vec![(generator_id, 500, 700)],
                    payloads,
                    ..Default::default()
                })
                .is_none(),
            "panning does not rescan the source when screen payloads are unchanged"
        );
        for sample in index.query(0..4, 450, 550, 1000) {
            assert_eq!(
                sample.row,
                samples
                    .iter()
                    .find(|s| s.representative.id == sample.representative.id)
                    .unwrap()
                    .row
            );
        }
    }

    #[test]
    fn transactions_are_sorted_by_time_even_when_ids_are_not() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("unordered.vtr");
        let mut writer = vtr::Writer::create(&path).unwrap();
        let stream = writer.add_stream(None, "packets", "CHI");
        let generator = writer.add_generator(stream, "packet");
        for (begin, end) in [(100, 110), (10, 200), (20, 25), (20, 30)] {
            let tx = writer.begin_tx(generator, begin).unwrap();
            writer.end_tx(tx, end, vtr::TxStatus::Ok).unwrap();
        }
        writer.close().unwrap();
        let container = to_ftr(&path).unwrap().unwrap();
        let transactions =
            &container.inner.tx_generators[&GeneratorId(generator.0 as usize)].transactions;
        assert_eq!(
            transactions
                .iter()
                .map(|tx| tx.get_tx_id().0)
                .collect::<Vec<_>>(),
            [2, 3, 4, 1]
        );
        assert_eq!(transactions[0].get_end_time(), 200u32.into());
    }

    #[test]
    fn chi_noc_overlapping_packets_have_distinct_canvas_rows() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/chi_noc.vtr");
        let container = to_ftr(&path).unwrap().unwrap();
        for stream in container.inner.tx_streams.values() {
            let assignments = crate::transactions::packet_rows(
                stream
                    .generators
                    .iter()
                    .flat_map(|id| container.inner.tx_generators[id].transactions.iter()),
            );
            let mut rows: HashMap<usize, Vec<_>> = HashMap::new();
            for id in &stream.generators {
                for tx in &container.inner.tx_generators[id].transactions {
                    rows.entry(assignments[&tx.get_tx_id()])
                        .or_default()
                        .push((tx.get_start_time(), tx.get_end_time()));
                }
            }
            assert_eq!(rows.len(), 64);
            for intervals in rows.values_mut() {
                intervals.sort();
                assert!(intervals.windows(2).all(|pair| pair[0].1 <= pair[1].0));
            }
        }
    }

    #[test]
    fn exports_vtr_transactions_for_the_existing_transaction_view() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/transactions.vtr");

        let container = to_ftr(&path).unwrap().unwrap();
        let stream = container.get_stream_from_name("cpu".into()).unwrap();
        let generator = container
            .get_generator_from_name(Some(stream.id), "issue".into())
            .unwrap();
        assert_eq!(generator.transactions.len(), 2);
        assert_eq!(generator.transactions[0].get_start_time(), 2u32.into());
        assert_eq!(generator.transactions[0].attributes[0].value(), "add");
        let details = container
            .vtr_details(generator.transactions[0].get_tx_id())
            .unwrap();
        assert_eq!(details.status, "ok");
        assert_eq!(details.kind, "internal");
        assert_eq!(details.events[0].name, "retire");
        assert_eq!(details.stages[0].lane, "alu");
        assert_eq!(
            details.relations[0].attrs[0],
            ("opcode".into(), "edge".into())
        );
    }
}
