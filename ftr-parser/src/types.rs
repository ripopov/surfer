use crate::ftr_parser::FtrParser;
use crate::types::DataType::Error;
use crate::types::Timescale::{Fs, Ms, Ns, Ps, Us, S};
use core::fmt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

type IsCompressed = bool;

pub type FtrResult<T> = Result<T, String>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockStatus {
    #[default]
    Indexed,
    Loaded,
    Error(String),
}

/// Seekable metadata for one transaction chunk. The offset points to the
/// chunk payload framing: the byte string for uncompressed chunks and the
/// uncompressed-size integer for compressed chunks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockMeta {
    pub stream_id: StreamId,
    pub ordinal: u64,
    pub encoded_offset: u64,
    pub encoded_len: u64,
    pub compressed: bool,
    pub uncompressed_len: Option<u64>,
    pub start_time: u64,
    pub end_time: u64,
    pub status: BlockStatus,
}

/// Seekable metadata for one relationship chunk. Offsets use the same
/// byte-string framing convention as [`BlockMeta`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationBlockMeta {
    pub ordinal: u64,
    pub encoded_offset: u64,
    pub encoded_len: u64,
    pub compressed: bool,
    pub uncompressed_len: Option<u64>,
    #[serde(default)]
    pub record_count: Option<u64>,
    pub status: BlockStatus,
}

// Dedicated ID types
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct StreamId(pub u64);

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct GeneratorId(pub u64);

impl fmt::Display for GeneratorId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct TransactionId(pub u64);

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct NameId(pub u64);

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
    /// Complete seekable transaction-block directory.
    #[serde(default)]
    pub tx_blocks: Vec<BlockMeta>,
    /// Legacy offset directory retained for state/test compatibility. New
    /// code uses `tx_blocks`.
    #[serde(default)]
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
    /// Immutable transaction body shared by asynchronous projections.
    ///
    /// The parser retains exclusive ownership while loading through
    /// [`Arc::make_mut`]. Once published, consumers can take a constant-time
    /// snapshot without cloning every transaction.
    pub transactions: Arc<Vec<Transaction>>,
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
    /// Immutable relation body shared by asynchronous projections.
    pub tx_relations: Arc<Vec<TxRelation>>,
    /// Complete seekable relationship-chunk directory for file-backed FTRs.
    #[serde(default)]
    pub relation_blocks: Vec<RelationBlockMeta>,
    /// Stable permutation of relation indices sorted by source transaction id
    /// and then recorded order. A binary-searched equal range replaces one
    /// heap allocation per transaction id.
    #[serde(skip)]
    pub(crate) rel_by_source: Vec<usize>,
    /// Stable permutation sorted by sink transaction id and recorded order.
    #[serde(skip)]
    pub(crate) rel_by_sink: Vec<usize>,
    pub(crate) path: Option<PathBuf>,
}

impl FTR {
    #[must_use]
    pub fn from_parts(
        time_scale: Timescale,
        max_timestamp: u64,
        str_dict: HashMap<NameId, Arc<str>>,
        tx_streams: HashMap<StreamId, TxStream>,
        tx_generators: HashMap<GeneratorId, TxGenerator>,
        tx_relations: Vec<TxRelation>,
    ) -> Self {
        let mut ftr = Self {
            time_scale,
            max_timestamp,
            str_dict,
            tx_streams,
            tx_generators,
            tx_relations: Arc::new(tx_relations),
            relation_blocks: Vec::new(),
            rel_by_source: Vec::new(),
            rel_by_sink: Vec::new(),
            path: None,
        };
        ftr.rebuild_relation_indices();
        ftr
    }

    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn rebuild_relation_indices(&mut self) {
        self.rel_by_source = (0..self.tx_relations.len()).collect();
        self.rel_by_source
            .sort_by_key(|index| (self.tx_relations[*index].source_tx_id, *index));
        self.rel_by_sink = (0..self.tx_relations.len()).collect();
        self.rel_by_sink
            .sort_by_key(|index| (self.tx_relations[*index].sink_tx_id, *index));
    }

    pub(crate) fn relations_from(&self, id: TransactionId) -> &[usize] {
        let start = self
            .rel_by_source
            .partition_point(|index| self.tx_relations[*index].source_tx_id < id);
        let end = self
            .rel_by_source
            .partition_point(|index| self.tx_relations[*index].source_tx_id <= id);
        &self.rel_by_source[start..end]
    }

    pub(crate) fn relations_to(&self, id: TransactionId) -> &[usize] {
        let start = self
            .rel_by_sink
            .partition_point(|index| self.tx_relations[*index].sink_tx_id < id);
        let end = self
            .rel_by_sink
            .partition_point(|index| self.tx_relations[*index].sink_tx_id <= id);
        &self.rel_by_sink[start..end]
    }

    // Takes a stream id and loads all associated transactions into memory
    pub fn load_stream_into_memory(&mut self, stream_id: StreamId) -> FtrResult<()> {
        self.ensure_relations_loaded()?;
        let mut ftr_parser = FtrParser::new(self);
        ftr_parser.load_transactions(stream_id)
    }

    /// Loads the file-backed relationship chunks without loading transaction
    /// bodies. Repeated calls are cheap while the body is resident.
    pub fn load_relations_into_memory(&mut self) -> FtrResult<()> {
        self.ensure_relations_loaded()
    }

    /// Releases the eager relation body while every transaction stream is
    /// still unloaded. File-backed compact projections can then own relation
    /// data only for the duration of their build instead of retaining a
    /// duplicate legacy graph in the UI container.
    pub fn release_relations_if_unloaded(&mut self) -> bool {
        if self.path.is_none()
            || self
                .tx_streams
                .values()
                .any(|stream| stream.transactions_loaded)
        {
            return false;
        }
        self.tx_relations = Arc::new(Vec::new());
        self.rel_by_source.clear();
        self.rel_by_sink.clear();
        for block in &mut self.relation_blocks {
            block.status = BlockStatus::Indexed;
        }
        true
    }

    fn ensure_relations_loaded(&mut self) -> FtrResult<()> {
        if self.relation_blocks.is_empty()
            || self
                .relation_blocks
                .iter()
                .all(|block| block.status == BlockStatus::Loaded)
        {
            return Ok(());
        }
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let mut parser = FtrParser::new(self);
        parser.load_relations_from_file(&path)
    }

    /// Visits file-backed transaction blocks in recorded order without
    /// populating `TxGenerator::transactions`.
    pub fn visit_stream_blocks<F>(&mut self, stream_id: StreamId, visit: F) -> FtrResult<()>
    where
        F: FnMut(&BlockMeta, &[Transaction]) -> FtrResult<()>,
    {
        self.ensure_relations_loaded()?;
        let mut parser = FtrParser::new(self);
        parser.visit_transaction_blocks(stream_id, visit)
    }

    /// Visits file-backed transaction blocks without populating per-record
    /// relation-index vectors. Compact projections join the relationship
    /// chunks separately and avoid materializing the global relation graph.
    pub fn visit_stream_blocks_unlinked<F>(
        &mut self,
        stream_id: StreamId,
        visit: F,
    ) -> FtrResult<()>
    where
        F: FnMut(&BlockMeta, &[Transaction]) -> FtrResult<()>,
    {
        let mut parser = FtrParser::new(self);
        parser.visit_transaction_blocks(stream_id, visit)
    }

    /// Visits relationship chunks in recorded order and releases each decoded
    /// batch after the callback returns.
    pub fn visit_relation_blocks<F>(&mut self, visit: F) -> FtrResult<()>
    where
        F: FnMut(&RelationBlockMeta, &[TxRelation]) -> FtrResult<()>,
    {
        let mut parser = FtrParser::new(self);
        parser.visit_relation_blocks(visit)
    }

    /// Decodes one seekable file-backed transaction block without retaining
    /// it in the generic transaction graph.
    pub fn read_stream_block(
        &mut self,
        stream_id: StreamId,
        ordinal: u64,
    ) -> FtrResult<Vec<Transaction>> {
        self.ensure_relations_loaded()?;
        let mut parser = FtrParser::new(self);
        parser.read_transaction_block(stream_id, ordinal)
    }

    /// Decodes one transaction block without constructing relation-index
    /// vectors. The caller joins independently paged relationship records.
    pub fn read_stream_block_unlinked(
        &mut self,
        stream_id: StreamId,
        ordinal: u64,
    ) -> FtrResult<Vec<Transaction>> {
        let mut parser = FtrParser::new(self);
        parser.read_transaction_block(stream_id, ordinal)
    }

    /// Decodes one relationship chunk without retaining it in [`FTR`].
    pub fn read_relation_block(&mut self, ordinal: u64) -> FtrResult<Vec<TxRelation>> {
        let mut parser = FtrParser::new(self);
        parser.read_relation_block(ordinal)
    }

    // drops all transactions from this stream from memory, but the stream itself doesn't get deleted
    pub fn drop_stream_from_memory(&mut self, stream_id: StreamId) {
        if let Some(stream) = self.tx_streams.get_mut(&stream_id) {
            for gen_id in &stream.generators {
                if let Some(gen) = self.tx_generators.get_mut(gen_id) {
                    Arc::make_mut(&mut gen.transactions).clear();
                }
            }
            stream.transactions_loaded = false;
            for block in &mut stream.tx_blocks {
                block.status = BlockStatus::Indexed;
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
