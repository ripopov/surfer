use crate::time::{TimeScale, TimeUnit};
use crate::wave_container::MetaData;
use ftr_parser::types::{
    FTR, GeneratorId, StreamId, Transaction, TransactionId, TxGenerator, TxStream,
};
use num::BigUint;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Not;

pub struct TransactionContainer {
    pub(crate) locations: std::sync::Mutex<HashMap<TransactionId, (GeneratorId, usize)>>,
    pub(crate) indexes: std::sync::Mutex<
        HashMap<
            crate::transaction_index::TrackKey,
            std::sync::Arc<crate::transaction_index::TrackIndex>,
        >,
    >,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) native: Option<crate::vtr_transactions::NativeTransactions>,
    pub inner: FTR,
    pub(crate) vtr_details: Option<HashMap<TransactionId, VtrTransactionDetails>>,
}

/// VTR fields that have no representation in the legacy FTR transaction type.
/// They stay attached to the immutable document for the inspector and future
/// native pipeline rendering.
#[derive(Clone, Debug)]
pub(crate) struct VtrTransactionDetails {
    pub status: String,
    pub kind: String,
    pub parent: Option<TransactionId>,
    pub events: Vec<VtrTransactionEvent>,
    pub stages: Vec<VtrTransactionStage>,
    pub relations: Vec<VtrTransactionRelation>,
}

#[derive(Clone, Debug)]
pub(crate) struct VtrTransactionEvent {
    pub time: u64,
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub(crate) struct VtrTransactionStage {
    pub name: String,
    pub lane: String,
    pub begin: u64,
    pub end: Option<u64>,
    pub attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub(crate) struct VtrTransactionRelation {
    pub name: String,
    pub source: TransactionId,
    pub target: TransactionId,
    pub attrs: Vec<(String, String)>,
}

impl TransactionContainer {
    pub(crate) fn track_index(
        &self,
        reference: &TransactionStreamRef,
    ) -> Option<std::sync::Arc<crate::transaction_index::TrackIndex>> {
        use crate::transaction_index::{Span, TrackIndex};
        use num::ToPrimitive;
        let key = reference.into();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(native) = &self.native {
            return native.tracks.get(&key).cloned();
        }
        if let Some(index) = self.indexes.lock().unwrap().get(&key) {
            return Some(index.clone());
        }
        let stream = self.get_stream(reference.stream_id)?;
        if !stream.transactions_loaded {
            return None;
        }
        let generators: Vec<_> = if let Some(generator) = reference.gen_id {
            vec![generator]
        } else {
            stream.generators.clone()
        };
        {
            let mut locations = self.locations.lock().unwrap();
            for generator in generators.iter().filter_map(|id| self.get_generator(*id)) {
                for (position, tx) in generator.transactions.iter().enumerate() {
                    locations.insert(tx.get_tx_id(), (generator.id, position));
                }
            }
        }
        let spans: Option<Vec<_>> = generators
            .iter()
            .filter_map(|id| self.get_generator(*id))
            .flat_map(|g| &g.transactions)
            .map(|tx| {
                Some(Span {
                    id: tx.get_tx_id().0 as u64,
                    generator: tx.get_gen_id().0 as u32,
                    begin: tx.get_start_time().to_u64()?,
                    end: tx.get_end_time().to_u64()?,
                })
            })
            .collect();
        let index = std::sync::Arc::new(TrackIndex::new(spans?));
        self.indexes.lock().unwrap().insert(key, index.clone());
        Some(index)
    }

    pub(crate) fn is_native(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.native.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
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
        if let Some((generator, position)) = self
            .locations
            .lock()
            .unwrap()
            .get(&transaction_ref.id)
            .copied()
        {
            return self.get_generator(generator)?.transactions.get(position);
        }
        self.inner.tx_generators.values().find_map(|g| {
            g.transactions
                .iter()
                .find(|tx| tx.get_tx_id() == transaction_ref.id)
        })
    }

    #[must_use]
    pub(crate) fn vtr_details(&self, id: TransactionId) -> Option<&VtrTransactionDetails> {
        self.vtr_details.as_ref()?.get(&id)
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
    pub fn get_transactions_from_generator(&self, gen_id: GeneratorId) -> Vec<TransactionId> {
        self.get_generator(gen_id)
            .into_iter()
            .flat_map(|generator| &generator.transactions)
            .map(Transaction::get_tx_id)
            .collect()
    }

    #[must_use]
    pub fn get_transactions_from_stream(&self, stream_id: StreamId) -> Vec<TransactionId> {
        self.get_stream(stream_id)
            .into_iter()
            .flat_map(|stream| &stream.generators)
            .filter_map(|id| self.get_generator(*id))
            .flat_map(|generator| &generator.transactions)
            .map(Transaction::get_tx_id)
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
        Some(BigUint::try_from(&self.inner.max_timestamp).unwrap())
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
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(native) = &self.native {
            return !native.in_flight && native.loaded == native.desired;
        }
        true
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
