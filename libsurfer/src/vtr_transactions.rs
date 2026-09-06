//! VTR transaction adapter.
//!
//! The transaction widgets currently consume `ftr_parser`'s in-memory model.
//! This adapter preserves VTR's stream/generator grouping, attributes and
//! relations at that boundary while the native VTR reader remains read-only.

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
}

impl TransactionDemand {
    pub fn normalize(&mut self) {
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

    fn contains(&self, tx: &FtrTransaction) -> bool {
        use num::ToPrimitive;
        self.pinned.contains(&(tx.event.tx_id.0 as u64))
            || self.windows.iter().any(|(generator, start, end)| {
                tx.event.gen_id.0 == *generator as usize
                    && tx.event.start_time.to_u64().unwrap_or(u64::MAX) <= *end
                    && tx.event.end_time.to_u64().unwrap_or(u64::MAX) >= *start
            })
    }
}

pub(crate) struct NativeTransactions {
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
    reader: std::sync::Arc<std::sync::Mutex<Reader>>,
    identity: u64,
    demand: TransactionDemand,
}

pub struct TransactionLoadResult {
    identity: u64,
    demand: TransactionDemand,
    data: Result<Option<TransactionContainer>, String>,
}

impl TransactionLoad {
    pub fn run(self) -> TransactionLoadResult {
        let queries: Vec<_> = self
            .demand
            .windows
            .iter()
            .map(|(generator, start, end)| TxQuery {
                generator: Some(vtr::NodeId(*generator)),
                window: Some((*start, *end)),
                ..Default::default()
            })
            .collect();
        let mut reader = self.reader.lock().unwrap();
        let data = from_reader(&reader, &queries, &self.demand.pinned);
        reader.clear_cache();
        TransactionLoadResult {
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
        if demand != native.desired {
            native.desired = demand;
            for generator in self.inner.tx_generators.values_mut() {
                generator
                    .transactions
                    .retain(|tx| native.desired.contains(tx));
            }
            let ids: std::collections::HashSet<_> = self
                .inner
                .tx_generators
                .values()
                .flat_map(|g| g.transactions.iter().map(|t| t.event.tx_id))
                .collect();
            if let Some(details) = &mut self.vtr_details {
                details.retain(|id, _| ids.contains(id));
            }
        }
        if native.desired.windows.is_empty() && native.desired.pinned.is_empty() {
            native.loaded = native.desired.clone();
            return None;
        }
        if native.in_flight || native.loaded == native.desired {
            return None;
        }
        native.in_flight = true;
        Some(TransactionLoad {
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
        if native.desired != result.demand {
            return false;
        }
        // A failed demand is not retried in a tight loop; changing demand permits a retry.
        native.loaded = result.demand;
        match result.data {
            Ok(Some(data)) => {
                self.inner = data.inner;
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
