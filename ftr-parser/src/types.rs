use crate::ftr_parser::FtrParser;
use crate::types::DataType::Error;
use crate::types::Timescale::{Fs, Ms, Ns, Ps, Us, S};
use core::fmt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

type IsCompressed = bool;

pub type FtrResult<T> = Result<T, String>;

// Dedicated ID types
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(pub usize);

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GeneratorId(pub usize);

impl fmt::Display for GeneratorId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransactionId(pub usize);

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NameId(pub usize);

impl fmt::Display for NameId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxStream {
    pub id: StreamId,
    pub name: String,
    pub kind: String,
    pub generators: Vec<GeneratorId>,
    pub transactions_loaded: bool,
    pub(super) tx_block_ids: Vec<(u64, IsCompressed)>,
}

impl PartialEq<Self> for TxStream {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxGenerator {
    pub id: GeneratorId,
    pub stream_id: StreamId,
    pub name: String,
    pub transactions: Vec<Transaction>,
}

impl PartialEq<Self> for TxGenerator {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.stream_id == other.stream_id
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TxRelation {
    /// Interned relation name, shared with the string dictionary.
    pub name: Arc<str>,
    pub source_tx_id: TransactionId,
    pub sink_tx_id: TransactionId,
    pub source_stream_id: StreamId,
    pub sink_stream_id: StreamId,
}

impl PartialEq<Self> for TxRelation {
    fn eq(&self, other: &Self) -> bool {
        self.source_tx_id == other.source_tx_id
            && self.sink_tx_id == other.sink_tx_id
            && self.source_stream_id == other.source_stream_id
            && self.sink_stream_id == other.sink_stream_id
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Transaction {
    pub event: Event,
    pub attributes: Vec<Attribute>,
    /// Indices into [`FTR::tx_relations`] for relations whose sink is this
    /// transaction (i.e. this transaction is the relation's target/child).
    /// Resolve them with [`FTR::get_relation`].
    pub inc_relations: Vec<usize>,
    /// Indices into [`FTR::tx_relations`] for relations whose source is this
    /// transaction (i.e. this transaction is the relation's origin/parent).
    /// Resolve them with [`FTR::get_relation`].
    pub out_relations: Vec<usize>,
    pub row: usize,
}

impl PartialEq<Self> for Transaction {
    fn eq(&self, other: &Self) -> bool {
        self.event.tx_id == other.event.tx_id && self.event.gen_id == other.event.gen_id
    }
}

impl Transaction {
    pub fn get_tx_id(&self) -> TransactionId {
        self.event.tx_id
    }

    pub fn get_gen_id(&self) -> GeneratorId {
        self.event.gen_id
    }

    pub fn get_start_time(&self) -> u64 {
        self.event.start_time
    }

    pub fn get_end_time(&self) -> u64 {
        self.event.end_time
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct Event {
    pub tx_id: TransactionId,
    pub gen_id: GeneratorId,
    pub start_time: u64,
    pub end_time: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribute {
    pub kind: AttributeType,
    /// Interned attribute name, shared with the string dictionary so repeated
    /// names (e.g. millions of `"name"` event attributes) are stored once.
    pub name: Arc<str>,
    pub data_type: DataType,
}

impl Attribute {
    pub fn new_empty() -> Self {
        let kind = AttributeType::NONE;
        let name = Arc::from("");
        let data_type = Error;
        Self {
            kind,
            name,
            data_type,
        }
    }

    pub fn new_begin(name: impl Into<Arc<str>>, data_type: DataType) -> Self {
        Self {
            kind: AttributeType::BEGIN,
            name: name.into(),
            data_type,
        }
    }

    pub fn new_record(name: impl Into<Arc<str>>, data_type: DataType) -> Self {
        Self {
            kind: AttributeType::RECORD,
            name: name.into(),
            data_type,
        }
    }

    pub fn new_end(name: impl Into<Arc<str>>, data_type: DataType) -> Self {
        Self {
            kind: AttributeType::END,
            name: name.into(),
            data_type,
        }
    }

    pub fn value(&self) -> String {
        match &self.data_type {
            DataType::Boolean(b) => b.to_string(),
            DataType::Enumeration(s) => s.to_string(),
            DataType::Integer(i) => i.to_string(),
            DataType::Unsigned(u) => u.to_string(),
            DataType::FloatingPointNumber(f) => f.to_string(),
            DataType::BitVector(s) => s.to_string(),
            DataType::LogicVector(s) => s.to_string(),
            DataType::FixedPointInteger(f) => f.to_string(),
            DataType::UnsignedFixedPointInteger(f) => f.to_string(),
            DataType::Pointer(u) => u.to_string(),
            DataType::String(s) => s.to_string(),
            DataType::Time(u) => u.to_string(),
            Error => "".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DataType {
    Boolean(bool),
    Enumeration(Arc<str>),
    Integer(i64),
    Unsigned(u64),
    FloatingPointNumber(f32),
    BitVector(Arc<str>),
    LogicVector(Arc<str>),
    FixedPointInteger(f32),
    UnsignedFixedPointInteger(f32),
    Pointer(u64),
    String(Arc<str>),
    Time(u64),
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttributeType {
    BEGIN,
    RECORD,
    END,
    NONE,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FTR {
    pub time_scale: Timescale,
    pub max_timestamp: u64,
    pub str_dict: HashMap<NameId, Arc<str>>,
    pub tx_streams: HashMap<StreamId, TxStream>,
    pub tx_generators: HashMap<GeneratorId, TxGenerator>,
    pub tx_relations: Vec<TxRelation>,
    /// Relation indices keyed by source (parent) transaction id. Built while
    /// relation chunks are parsed and used to attach relations to transactions
    /// in O(1) instead of scanning the whole relation list per transaction.
    #[serde(skip)]
    pub(crate) rel_by_source: HashMap<TransactionId, Vec<usize>>,
    /// Relation indices keyed by sink (child) transaction id.
    #[serde(skip)]
    pub(crate) rel_by_sink: HashMap<TransactionId, Vec<usize>>,
    pub(crate) path: Option<PathBuf>,
}

impl FTR {
    // Takes a stream id and loads all associated transactions into memory
    pub fn load_stream_into_memory(&mut self, stream_id: StreamId) -> FtrResult<()> {
        let mut ftr_parser = FtrParser::new(self);
        ftr_parser.load_transactions(stream_id)
    }

    // drops all transactions from this stream from memory, but the stream itself doesn't get deleted
    pub fn drop_stream_from_memory(&mut self, stream_id: StreamId) {
        if let Some(stream) = self.tx_streams.get(&stream_id) {
            for gen_id in &stream.generators {
                if let Some(gen) = self.tx_generators.get_mut(gen_id) {
                    gen.transactions.clear();
                }
            }
        }
    }

    pub fn get_stream(&self, stream_id: StreamId) -> Option<&TxStream> {
        self.tx_streams.get(&stream_id)
    }

    pub fn get_stream_from_name(&self, name: String) -> Option<&TxStream> {
        self.tx_streams.values().find(|t| t.name == name)
    }

    pub fn get_generator(&self, gen_id: GeneratorId) -> Option<&TxGenerator> {
        self.tx_generators.get(&gen_id)
    }

    /// Resolves a relation index (as stored in a transaction's `inc_relations`
    /// or `out_relations`) to the corresponding [`TxRelation`].
    pub fn get_relation(&self, index: usize) -> Option<&TxRelation> {
        self.tx_relations.get(index)
    }

    /// Returns the `Optional<TxGenerator>` with the name `gen_name` from the stream with id `stream_id`.
    pub fn get_generator_from_name(
        &self,
        stream_id: Option<StreamId>,
        gen_name: String,
    ) -> Option<&TxGenerator> {
        if let Some(stream_id) = stream_id {
            self.tx_streams
                .get(&stream_id)?
                .generators
                .iter()
                .map(|id| self.tx_generators.get(id))
                .find(|gen| gen.is_some_and(|gen| gen.name == gen_name))?
        } else {
            self.tx_generators.values().find(|gen| gen.name == gen_name)
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum Timescale {
    Fs,
    Ps,
    Ns,
    Us,
    Ms,
    S,
    Unit,
    #[default]
    None,
}

impl Timescale {
    pub fn get_timescale(exponent: i64) -> Timescale {
        match exponent {
            0 => S,
            -4 => Ms,
            -8 => Us,
            -12 => Ns,
            -16 => Ps,
            -20 => Fs,
            _ => Timescale::None,
        }
    }
}

impl fmt::Display for Timescale {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
