use crate::time::{TimeScale, TimeUnit};
use crate::transaction_events::{EventIndex, EventInfo};
use crate::wave_container::MetaData;
use ftr_parser::types::{
    FTR, GeneratorId, StreamId, Transaction, TransactionId, TxGenerator, TxRelation, TxStream,
};
use itertools::Itertools;
use num::BigUint;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Not;

pub struct TransactionContainer {
    pub inner: FTR,
    /// Index over the FTR event convention, rebuilt when streams are loaded.
    /// Boxed to keep the container (and the messages carrying it) small.
    event_index: Box<EventIndex>,
}

impl TransactionContainer {
    #[must_use]
    pub fn new(inner: FTR) -> Self {
        let event_index = Box::new(EventIndex::build(&inner));
        TransactionContainer { inner, event_index }
    }

    /// Loads a stream's transactions into memory and refreshes the event
    /// index. All stream loading must go through here so the index stays
    /// consistent with the loaded data.
    pub fn load_stream(&mut self, stream_id: StreamId) -> Result<(), String> {
        self.inner.load_stream_into_memory(stream_id)?;
        *self.event_index = EventIndex::build(&self.inner);
        Ok(())
    }

    /// The event structure index for this trace.
    #[must_use]
    pub fn event_index(&self) -> &EventIndex {
        &self.event_index
    }

    /// Parent info for an event transaction; `None` for non-events and
    /// orphan events.
    #[must_use]
    pub fn event_info(&self, tx_id: TransactionId) -> Option<&EventInfo> {
        self.event_index.event_info(tx_id)
    }

    /// Events of a parent transaction, sorted by (start time, id).
    #[must_use]
    pub fn events_of_parent(&self, tx_id: TransactionId) -> &[TransactionId] {
        self.event_index.events_of_parent(tx_id)
    }

    /// Resolves a relation index (as stored in a transaction's
    /// `inc_relations`/`out_relations`) to the relation itself.
    #[must_use]
    pub fn get_relation(&self, index: usize) -> Option<&TxRelation> {
        self.inner.get_relation(index)
    }
    #[must_use]
    pub fn get_streams(&self) -> Vec<&TxStream> {
        self.inner.tx_streams.values().collect()
    }

    #[must_use]
    pub fn get_stream(&self, stream_id: StreamId) -> Option<&TxStream> {
        self.inner.get_stream(stream_id)
    }

    #[must_use]
    pub fn get_stream_from_name(&self, name: String) -> Option<&TxStream> {
        self.inner.get_stream_from_name(name)
    }

    #[must_use]
    pub fn get_generators(&self) -> Vec<&TxGenerator> {
        self.inner.tx_generators.values().collect()
    }

    #[must_use]
    pub fn get_transaction(&self, transaction_ref: &TransactionRef) -> Option<&Transaction> {
        // O(1) via the event index lookup, falling back to a scan in case
        // the index has not seen the transaction (e.g. mid-load)
        self.event_index
            .lookup_tx(transaction_ref.id)
            .and_then(|(gen_id, idx)| self.inner.tx_generators.get(&gen_id)?.transactions.get(idx))
            .filter(|tx| tx.get_tx_id() == transaction_ref.id)
            .or_else(|| {
                self.inner.tx_generators.values().find_map(|g| {
                    g.transactions
                        .iter()
                        .find(|tx| tx.get_tx_id() == transaction_ref.id)
                })
            })
    }

    #[must_use]
    pub fn get_generator(&self, gen_id: GeneratorId) -> Option<&TxGenerator> {
        self.inner.get_generator(gen_id)
    }
    #[must_use]
    pub fn get_generator_from_name(
        &self,
        stream_id: Option<StreamId>,
        gen_name: String,
    ) -> Option<&TxGenerator> {
        self.inner.get_generator_from_name(stream_id, gen_name)
    }

    #[must_use]
    pub fn get_generator_from_name_or_qualified(
        &self,
        stream_id: Option<StreamId>,
        gen_name: &str,
    ) -> Option<&TxGenerator> {
        self.get_generator_from_name(stream_id, gen_name.to_string())
            .or_else(|| match stream_id {
                Some(stream_id) => {
                    let (_, name) = gen_name.rsplit_once('.')?;
                    self.get_generator_from_name(Some(stream_id), name.to_string())
                }
                None => {
                    let (stream_name, name) = gen_name.rsplit_once('.')?;
                    let stream = self
                        .get_stream_from_name(stream_name.to_string())
                        .or_else(|| self.get_stream_from_name(format!("tr.{stream_name}")))?;
                    self.get_generator_from_name(Some(stream.id), name.to_string())
                }
            })
    }

    #[must_use]
    pub fn get_transactions_from_generator(&self, gen_id: GeneratorId) -> Vec<TransactionId> {
        self.get_generator(gen_id)
            .unwrap()
            .transactions
            .iter()
            .map(ftr_parser::types::Transaction::get_tx_id)
            .collect_vec()
    }

    #[must_use]
    pub fn get_transactions_from_stream(&self, stream_id: StreamId) -> Vec<TransactionId> {
        self.get_stream(stream_id)
            .unwrap()
            .generators
            .iter()
            .flat_map(|g| {
                self.get_generator(*g)
                    .unwrap()
                    .transactions
                    .iter()
                    .map(ftr_parser::types::Transaction::get_tx_id)
                    .collect_vec()
            })
            .collect()
    }
    #[must_use]
    pub fn stream_scope_exists(&self, stream_scope: &StreamScopeRef) -> bool {
        match stream_scope {
            StreamScopeRef::Root => true,
            StreamScopeRef::Stream(s) => self.inner.tx_streams.contains_key(&s.stream_id),
            StreamScopeRef::Empty(_) => false,
        }
    }

    #[must_use]
    pub fn stream_names(&self) -> Vec<String> {
        let mut names = vec![String::from("tr")];
        let mut stream_names: Vec<String> = self
            .get_streams()
            .into_iter()
            .map(|s| s.name.clone())
            .collect();
        names.append(&mut stream_names);

        names
    }

    #[must_use]
    pub fn generator_names(&self) -> Vec<String> {
        self.get_generators()
            .into_iter()
            .map(|g| g.name.clone())
            .collect()
    }

    #[must_use]
    pub fn generators_in_stream(&self, stream_scope: &StreamScopeRef) -> Vec<TransactionStreamRef> {
        match stream_scope {
            StreamScopeRef::Root => self
                .get_streams()
                .into_iter()
                .map(|s| TransactionStreamRef {
                    gen_id: None,
                    stream_id: s.id,
                    name: s.name.clone(),
                })
                .collect(),
            StreamScopeRef::Stream(stream_ref) => self
                .get_stream(stream_ref.stream_id)
                .unwrap()
                .generators
                .iter()
                .map(|id| {
                    let generator = self.get_generator(*id).unwrap();
                    TransactionStreamRef {
                        gen_id: Some(generator.id),
                        stream_id: stream_ref.stream_id,
                        name: generator.name.clone(),
                    }
                })
                .collect(),
            StreamScopeRef::Empty(_) => vec![],
        }
    }

    #[must_use]
    pub fn max_timestamp(&self) -> Option<BigUint> {
        Some(BigUint::from(self.inner.max_timestamp))
    }

    #[must_use]
    pub fn metadata(&self) -> MetaData {
        MetaData {
            date: None,
            version: None,
            timescale: TimeScale {
                unit: TimeUnit::from(self.inner.time_scale),
                multiplier: None,
            },
        }
    }

    #[must_use]
    pub fn body_loaded(&self) -> bool {
        true // for now
    }

    #[must_use]
    pub fn is_fully_loaded(&self) -> bool {
        true // for now
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StreamScopeRef {
    Root,
    Stream(TransactionStreamRef),
    Empty(String),
}

impl Display for StreamScopeRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamScopeRef::Root => write!(f, "Root scope"),
            StreamScopeRef::Stream(s) => s.fmt(f),
            StreamScopeRef::Empty(_) => write!(f, "Empty stream scope"),
        }
    }
}

impl StreamScopeRef {
    #[must_use]
    pub fn new_stream_from_name(transactions: &TransactionContainer, name: String) -> Self {
        let stream = transactions
            .inner
            .get_stream_from_name(name.clone())
            .unwrap();
        StreamScopeRef::Stream(TransactionStreamRef::new_stream(stream.id, name))
    }
}

/// If `gen_id` is `Some` this `TransactionStreamRef` is a generator, otherwise it's a stream
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionStreamRef {
    pub stream_id: StreamId,
    pub gen_id: Option<GeneratorId>,
    pub name: String,
}

impl Hash for TransactionStreamRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.gen_id
            .unwrap_or(GeneratorId(self.stream_id.0))
            .hash(state);
        self.name.hash(state);
    }
}

impl Display for TransactionStreamRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_generator() {
            write!(
                f,
                "Generator: id: {}, stream_id: {}, name: {}",
                self.gen_id.unwrap(),
                self.stream_id,
                self.name
            )
        } else {
            write!(f, "Stream: id: {}, name: {}", self.stream_id, self.name)
        }
    }
}

impl TransactionStreamRef {
    #[must_use]
    pub fn new_stream(stream_id: StreamId, name: String) -> Self {
        TransactionStreamRef {
            stream_id,
            gen_id: None,
            name,
        }
    }
    #[must_use]
    pub fn new_gen(stream_id: StreamId, gen_id: GeneratorId, name: String) -> Self {
        TransactionStreamRef {
            stream_id,
            gen_id: Some(gen_id),
            name,
        }
    }

    #[must_use]
    pub fn is_generator(&self) -> bool {
        self.gen_id.is_some()
    }

    #[must_use]
    pub fn is_stream(&self) -> bool {
        self.is_generator().not()
    }
}

#[derive(Clone, Debug, Eq, Hash, Serialize, Deserialize, PartialEq)]
pub struct TransactionRef {
    pub id: TransactionId,
}
