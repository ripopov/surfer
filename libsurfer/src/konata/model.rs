use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ftr_parser::types::{
    Attribute, AttributeType, DataType, GeneratorId, StreamId, Transaction, TxRelation,
};
use serde::{Deserialize, Serialize};

use crate::transaction_events::{EVENT_NAME_ATTRIBUTE, EVENT_PARENT_RELATION};

use super::{
    KonataAnnotationDetail, KonataDetailCacheTelemetry, KonataDetailQuery, KonataRowDetail,
    detail::KonataDetailStore,
};

const MISSING_U64: u64 = u64::MAX;
const MISSING_U32: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushState {
    False,
    True,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowFlags(pub u16);

impl RowFlags {
    pub const BEGIN_REGRESSION: u16 = 1 << 0;
    pub const MISSING_RID: u16 = 1 << 1;
    pub const DUPLICATE_RID: u16 = 1 << 2;

    fn insert(&mut self, flag: u16) {
        self.0 |= flag;
    }

    #[must_use]
    pub fn contains(self, flag: u16) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageFlags(pub u8);

impl StageFlags {
    pub const OUT_OF_RANGE: u8 = 1 << 0;
    pub const END_BEFORE_START: u8 = 1 << 1;
    pub const UNNAMED: u8 = 1 << 2;
    pub const MULTIPLE_PARENTS: u8 = 1 << 3;

    fn insert(&mut self, flag: u8) {
        self.0 |= flag;
    }

    #[must_use]
    pub fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KonataQualityCounters {
    pub orphans: u64,
    pub multiple_parents: u64,
    pub unnamed_stages: u64,
    pub out_of_range: u64,
    pub end_before_start: u64,
    pub begin_regressions: u64,
    pub unknown_lanes: u64,
    pub missing_rid: u64,
    pub duplicate_rid: u64,
}

impl KonataQualityCounters {
    #[must_use]
    pub fn total(&self) -> u64 {
        self.orphans
            + self.multiple_parents
            + self.unnamed_stages
            + self.out_of_range
            + self.end_before_start
            + self.begin_regressions
            + self.unknown_lanes
            + self.missing_rid
            + self.duplicate_rid
    }
}

#[derive(Debug, Clone)]
pub struct InstructionColumns {
    pub begin: Vec<u64>,
    pub end: Vec<u64>,
    pub tx_id: Vec<u64>,
    pub sid: Vec<u64>,
    pub rid: Vec<u64>,
    pub tid: Vec<u32>,
    pub flags: Vec<RowFlags>,
    pub flushed: Vec<FlushState>,
    pub label: Vec<u32>,
    pub detail: Vec<u32>,
}

impl InstructionColumns {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            begin: Vec::with_capacity(capacity),
            end: Vec::with_capacity(capacity),
            tx_id: Vec::with_capacity(capacity),
            sid: Vec::with_capacity(capacity),
            rid: Vec::with_capacity(capacity),
            tid: Vec::with_capacity(capacity),
            flags: Vec::with_capacity(capacity),
            flushed: Vec::with_capacity(capacity),
            label: Vec::with_capacity(capacity),
            detail: Vec::with_capacity(capacity),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.begin.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.begin.is_empty()
    }

    #[must_use]
    pub fn sid(&self, row: usize) -> Option<u64> {
        (self.sid[row] != MISSING_U64).then_some(self.sid[row])
    }

    #[must_use]
    pub fn rid(&self, row: usize) -> Option<u64> {
        (self.rid[row] != MISSING_U64).then_some(self.rid[row])
    }

    #[must_use]
    pub fn tid(&self, row: usize) -> Option<u32> {
        (self.tid[row] != MISSING_U32).then_some(self.tid[row])
    }

    #[must_use]
    pub fn label(&self, row: usize) -> Option<u32> {
        (self.label[row] != MISSING_U32).then_some(self.label[row])
    }

    #[must_use]
    pub fn detail(&self, row: usize) -> Option<u32> {
        (self.detail[row] != MISSING_U32).then_some(self.detail[row])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum KonataScalar {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f32),
    Text(u32),
    Time(u64),
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KonataAnnotation {
    pub name: u32,
    pub value: KonataScalar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KonataStage {
    pub start: u64,
    pub end: u64,
    pub event_tx: u64,
    pub name: u16,
    pub lane: u16,
    pub flags: StageFlags,
    pub detail_page: u32,
    pub annotation_start: u32,
    pub annotation_end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonataDependency {
    pub producer_row: u32,
    pub consumer_row: u32,
    pub name: u16,
    /// Exact endpoint timestamp when the relation names a stage event.
    pub producer_tick: Option<u64>,
    pub consumer_tick: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonataSearchHits {
    bits: Vec<u64>,
    ranks: Vec<u32>,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonataRowSet {
    bits: Vec<u64>,
    count: usize,
}

impl KonataRowSet {
    fn empty(row_count: usize) -> Self {
        Self {
            bits: vec![0; row_count.div_ceil(64)],
            count: 0,
        }
    }

    fn insert(&mut self, row: usize) -> bool {
        let bit = &mut self.bits[row / 64];
        let mask = 1 << (row % 64);
        if *bit & mask != 0 {
            return false;
        }
        *bit |= mask;
        self.count += 1;
        true
    }

    #[must_use]
    pub fn contains(&self, row: usize) -> bool {
        self.bits
            .get(row / 64)
            .is_some_and(|word| *word & (1 << (row % 64)) != 0)
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }
}

impl KonataSearchHits {
    #[must_use]
    pub fn empty(row_count: usize) -> Self {
        Self {
            bits: vec![0; row_count.div_ceil(64)],
            ranks: Vec::new(),
            count: 0,
        }
    }

    pub fn finish(&mut self) {
        let mut count = 0u32;
        self.ranks = self
            .bits
            .iter()
            .map(|word| {
                let before = count;
                count += word.count_ones();
                before
            })
            .collect();
        self.count = count as usize;
    }

    pub fn insert(&mut self, row: usize) {
        let bit = &mut self.bits[row / 64];
        let mask = 1 << (row % 64);
        if *bit & mask == 0 {
            *bit |= mask;
            self.count += 1;
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn contains(&self, row: usize) -> bool {
        self.bits
            .get(row / 64)
            .is_some_and(|word| *word & (1 << (row % 64)) != 0)
    }

    #[must_use]
    pub fn next(&self, row: usize, reverse: bool) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        if reverse {
            let rank = self.rank_before(row);
            self.select(if rank == 0 { self.count - 1 } else { rank - 1 })
        } else {
            let rank = self.rank_before(row.saturating_add(1));
            self.select(if rank == self.count { 0 } else { rank })
        }
    }

    #[must_use]
    pub fn first(&self) -> Option<usize> {
        self.select(0)
    }

    #[must_use]
    pub fn ordinal(&self, row: usize) -> Option<usize> {
        self.contains(row).then(|| self.rank_before(row) + 1)
    }

    fn rank_before(&self, row: usize) -> usize {
        let word = row / 64;
        if word >= self.bits.len() {
            return self.count;
        }
        let bit = row % 64;
        let prefix = if bit == 0 { 0 } else { (1u64 << bit) - 1 };
        self.ranks.get(word).copied().unwrap_or_default() as usize
            + (self.bits[word] & prefix).count_ones() as usize
    }

    fn select(&self, rank: usize) -> Option<usize> {
        if rank >= self.count {
            return None;
        }
        let word_index = self
            .ranks
            .partition_point(|before| *before as usize <= rank)
            .saturating_sub(1);
        let mut word = self.bits[word_index];
        let mut remaining = rank - self.ranks[word_index] as usize;
        while remaining > 0 {
            word &= word - 1;
            remaining -= 1;
        }
        Some(word_index * 64 + word.trailing_zeros() as usize)
    }
}

#[derive(Debug, Clone)]
pub struct KonataBuildInput {
    pub parent_generator: GeneratorId,
    pub event_generator: GeneratorId,
    pub stream: StreamId,
    pub parents: Arc<Vec<Transaction>>,
    pub events: Arc<Vec<Transaction>>,
    pub relations: Arc<Vec<TxRelation>>,
}

#[derive(Debug, Clone, Copy)]
struct ProjectedEventParent {
    first_row: u32,
    count: u32,
}

impl Default for ProjectedEventParent {
    fn default() -> Self {
        Self {
            first_row: MISSING_U32,
            count: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct ProjectedDependency {
    input_order: u64,
    name: Arc<str>,
    source_tx: u64,
    sink_tx: u64,
}

/// Incremental relation join used by paged local and remote model builders.
/// Event-parent relations collapse to one row plus a count; only relations
/// that can become visible dependency edges remain resident.
#[derive(Debug)]
pub struct KonataRelationProjector {
    stream: StreamId,
    parent_rows: Vec<(u64, u32)>,
    event_indices: Vec<(u64, u32)>,
    event_parents: Vec<ProjectedEventParent>,
    dependencies: Vec<ProjectedDependency>,
    next_input_order: u64,
}

#[derive(Debug)]
pub struct KonataRelationProjection {
    event_parents: Vec<ProjectedEventParent>,
    dependencies: Vec<ProjectedDependency>,
}

fn sorted_id_lookup(index: &[(u64, u32)], transaction: u64) -> Option<u32> {
    index
        .binary_search_by_key(&transaction, |(candidate, _)| *candidate)
        .ok()
        .map(|position| index[position].1)
}

impl KonataRelationProjector {
    #[must_use]
    pub fn new(parents: &[Transaction], events: &[Transaction], stream: StreamId) -> Self {
        let parent_ids = parents
            .iter()
            .map(|transaction| transaction.get_tx_id().0)
            .collect::<Vec<_>>();
        let event_ids = events
            .iter()
            .map(|transaction| transaction.get_tx_id().0)
            .collect::<Vec<_>>();
        Self::from_ids(&parent_ids, &event_ids, stream)
    }

    #[must_use]
    pub fn from_ids(parent_ids: &[u64], event_ids: &[u64], stream: StreamId) -> Self {
        let mut parent_rows = parent_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(row, transaction)| (transaction, row as u32))
            .collect::<Vec<_>>();
        parent_rows.sort_unstable();
        let mut event_indices = event_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, transaction)| (transaction, index as u32))
            .collect::<Vec<_>>();
        event_indices.sort_unstable();
        Self {
            stream,
            parent_rows,
            event_indices,
            event_parents: vec![ProjectedEventParent::default(); event_ids.len()],
            dependencies: Vec::new(),
            next_input_order: 0,
        }
    }

    pub fn push(&mut self, relations: &[TxRelation]) {
        for relation in relations {
            let input_order = self.next_input_order;
            self.next_input_order = self.next_input_order.wrapping_add(1);

            if relation.name.as_ref() == EVENT_PARENT_RELATION
                && relation.source_stream_id == self.stream
                && let Some(event) = sorted_id_lookup(&self.event_indices, relation.sink_tx_id.0)
            {
                if let Some(row) = sorted_id_lookup(&self.parent_rows, relation.source_tx_id.0) {
                    let projected = &mut self.event_parents[event as usize];
                    if projected.count == 0 {
                        projected.first_row = row;
                    }
                    projected.count = projected.count.saturating_add(1);
                }
                continue;
            }

            if relation.source_stream_id != self.stream || relation.sink_stream_id != self.stream {
                continue;
            }
            let source_known = sorted_id_lookup(&self.parent_rows, relation.source_tx_id.0)
                .or_else(|| sorted_id_lookup(&self.event_indices, relation.source_tx_id.0))
                .is_some();
            let sink_known = sorted_id_lookup(&self.parent_rows, relation.sink_tx_id.0)
                .or_else(|| sorted_id_lookup(&self.event_indices, relation.sink_tx_id.0))
                .is_some();
            if source_known && sink_known {
                self.dependencies.push(ProjectedDependency {
                    input_order,
                    name: relation.name.clone(),
                    source_tx: relation.source_tx_id.0,
                    sink_tx: relation.sink_tx_id.0,
                });
            }
        }
    }

    #[must_use]
    pub fn finish(self) -> KonataRelationProjection {
        KonataRelationProjection {
            event_parents: self.event_parents,
            dependencies: self.dependencies,
        }
    }
}

struct PendingStage {
    start: u64,
    end: u64,
    event_tx: u64,
    name: u16,
    lane: u16,
    unnamed: bool,
    unknown_lane: bool,
    annotation_start: u32,
    annotation_end: u32,
}

/// Incremental transaction normalizer for file/remote block streams. It owns
/// only the canonical row columns and compact pending-stage records; decoded
/// generic transaction batches can be dropped after each `push` call.
pub struct KonataRecordProjector {
    parent_generator: GeneratorId,
    event_generator: GeneratorId,
    stream: StreamId,
    strings: StringInterner,
    thread_keys: KeyInterner,
    rows: InstructionColumns,
    instruction_annotations: Vec<KonataAnnotation>,
    instruction_annotation_offsets: Vec<u32>,
    quality: KonataQualityCounters,
    previous_begin: Option<u64>,
    pending_stages: Vec<PendingStage>,
    pending_annotations: Vec<KonataAnnotation>,
    stage_name_ids: HashMap<u32, u16>,
    stage_names: Vec<u32>,
    lane_keys: KeyInterner,
    default_lane: u16,
}

impl KonataRecordProjector {
    #[must_use]
    pub fn new(
        parent_generator: GeneratorId,
        event_generator: GeneratorId,
        stream: StreamId,
        capacity: usize,
    ) -> Self {
        let mut strings = StringInterner::default();
        let mut lane_keys = KeyInterner::default();
        let default_lane = lane_keys.intern(TypedKey::Unsigned(0), &mut strings) as u16;
        Self {
            parent_generator,
            event_generator,
            stream,
            strings,
            thread_keys: KeyInterner::default(),
            rows: InstructionColumns::with_capacity(capacity),
            instruction_annotations: Vec::new(),
            instruction_annotation_offsets: vec![0],
            quality: KonataQualityCounters::default(),
            previous_begin: None,
            pending_stages: Vec::new(),
            pending_annotations: Vec::new(),
            stage_name_ids: HashMap::new(),
            stage_names: Vec::new(),
            lane_keys,
            default_lane,
        }
    }

    pub fn push(&mut self, transactions: &[Transaction]) {
        for transaction in transactions {
            let generator = transaction.get_gen_id();
            if generator == self.parent_generator {
                self.push_parent(transaction);
            } else if generator == self.event_generator {
                self.push_event(transaction);
            }
        }
    }

    pub async fn push_cooperative(&mut self, transactions: &[Transaction]) {
        for batch in transactions.chunks(1024) {
            self.push(batch);
            crate::async_util::sleep_ms(0).await;
        }
    }

    fn push_parent(&mut self, transaction: &Transaction) {
        let begin = transaction.get_start_time();
        let mut flags = RowFlags::default();
        if self.previous_begin.is_some_and(|previous| begin < previous) {
            flags.insert(RowFlags::BEGIN_REGRESSION);
            self.quality.begin_regressions += 1;
        }
        self.previous_begin = Some(begin);

        let sid = attribute(transaction, "insn_id_in_sim")
            .and_then(unsigned_value)
            .unwrap_or(MISSING_U64);
        let tid = attribute(transaction, "thread_id")
            .and_then(TypedKey::from_data)
            .map_or(MISSING_U32, |key| {
                self.thread_keys.intern(key, &mut self.strings)
            });
        let flushed = match attribute(transaction, "flushed") {
            Some(DataType::Boolean(true)) => FlushState::True,
            Some(DataType::Boolean(false)) => FlushState::False,
            _ => FlushState::Unknown,
        };
        let rid = (flushed != FlushState::True)
            .then(|| attribute(transaction, "retire_id").and_then(unsigned_value))
            .flatten()
            .unwrap_or(MISSING_U64);
        if rid == MISSING_U64 && flushed != FlushState::True {
            flags.insert(RowFlags::MISSING_RID);
            self.quality.missing_rid += 1;
        }
        let label = attribute(transaction, "label")
            .map_or(MISSING_U32, |value| self.strings.intern_data(value));
        let detail = attribute(transaction, "detail")
            .map_or(MISSING_U32, |value| self.strings.intern_data(value));
        self.instruction_annotations.extend(
            transaction
                .attributes
                .iter()
                .filter(|attribute| !is_projected_instruction_attribute(&attribute.name))
                .map(|attribute| KonataAnnotation {
                    name: self.strings.intern(attribute.name.clone()),
                    value: scalar_from_data(&attribute.data_type, &mut self.strings),
                }),
        );
        self.instruction_annotation_offsets
            .push(self.instruction_annotations.len() as u32);

        self.rows.begin.push(begin);
        self.rows.end.push(transaction.get_end_time());
        self.rows.tx_id.push(transaction.get_tx_id().0);
        self.rows.sid.push(sid);
        self.rows.rid.push(rid);
        self.rows.tid.push(tid);
        self.rows.flags.push(flags);
        self.rows.flushed.push(flushed);
        self.rows.label.push(label);
        self.rows.detail.push(detail);
    }

    fn push_event(&mut self, event: &Transaction) {
        let (name_string, unnamed) = match event_name_attribute(event) {
            Some(attribute) => (self.strings.intern_data(&attribute.data_type), false),
            None => (self.strings.intern(Arc::from("<unnamed stage>")), true),
        };
        let next_name = u16::try_from(self.stage_names.len()).unwrap_or(u16::MAX);
        let name = *self.stage_name_ids.entry(name_string).or_insert_with(|| {
            self.stage_names.push(name_string);
            next_name
        });
        let (lane, unknown_lane) = match attribute(event, "lane") {
            None => (self.default_lane, false),
            Some(value) => match TypedKey::from_data(value) {
                Some(key) => (self.lane_keys.intern(key, &mut self.strings) as u16, false),
                None => (
                    self.lane_keys.intern(
                        TypedKey::Text(Arc::from(format!("<unknown lane {}>", format_data(value)))),
                        &mut self.strings,
                    ) as u16,
                    true,
                ),
            },
        };
        let annotation_start = self.pending_annotations.len() as u32;
        self.pending_annotations.extend(
            event
                .attributes
                .iter()
                .filter(|attribute| {
                    attribute.name.as_ref() != EVENT_NAME_ATTRIBUTE
                        && (attribute.name.as_ref() != "lane"
                            || TypedKey::from_data(&attribute.data_type).is_none())
                })
                .map(|attribute| KonataAnnotation {
                    name: self.strings.intern(attribute.name.clone()),
                    value: scalar_from_data(&attribute.data_type, &mut self.strings),
                }),
        );
        self.pending_stages.push(PendingStage {
            start: event.get_start_time(),
            end: event.get_end_time(),
            event_tx: event.get_tx_id().0,
            name,
            lane,
            unnamed,
            unknown_lane,
            annotation_start,
            annotation_end: self.pending_annotations.len() as u32,
        });
    }

    #[must_use]
    pub fn relation_projector(&self) -> KonataRelationProjector {
        let mut parent_rows = self
            .rows
            .tx_id
            .iter()
            .copied()
            .enumerate()
            .map(|(row, transaction)| (transaction, row as u32))
            .collect::<Vec<_>>();
        parent_rows.sort_unstable();
        let mut event_indices = self
            .pending_stages
            .iter()
            .enumerate()
            .map(|(index, stage)| (stage.event_tx, index as u32))
            .collect::<Vec<_>>();
        event_indices.sort_unstable();
        KonataRelationProjector {
            stream: self.stream,
            parent_rows,
            event_indices,
            event_parents: vec![ProjectedEventParent::default(); self.pending_stages.len()],
            dependencies: Vec::new(),
            next_input_order: 0,
        }
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn event_count(&self) -> usize {
        self.pending_stages.len()
    }

    pub async fn finish(
        self,
        projection: KonataRelationProjection,
        cooperative: bool,
    ) -> KonataModel {
        KonataModel::build_records_inner(self, projection, cooperative).await
    }
}

#[derive(Debug, Clone)]
pub struct KonataModel {
    pub rows: InstructionColumns,
    pub strings: Vec<Arc<str>>,
    pub stage_names: Vec<u32>,
    pub thread_names: Vec<u32>,
    pub lane_names: Vec<u32>,
    pub dependency_names: Vec<u32>,
    pub dependencies: Vec<KonataDependency>,
    pub quality: KonataQualityCounters,
    pub visibility: VisibilityIndex,
    pub flushed_count: usize,
    all_ranges: BlockRangeIndex,
    visible_ranges: BlockRangeIndex,
    tx_to_row: Vec<(u64, u32)>,
    event_to_row: Vec<(u64, u32)>,
    sid_to_row: Vec<(u64, u32)>,
    rid_to_row: Vec<(u64, u32)>,
    thread_rid_to_row: Vec<(u32, u64, u32)>,
    time_to_row: Vec<(u64, u32)>,
    producer_offsets: Vec<u32>,
    producer_edges: Vec<u32>,
    consumer_offsets: Vec<u32>,
    consumer_edges: Vec<u32>,
    details: Arc<KonataDetailStore>,
}

impl KonataModel {
    #[must_use]
    pub fn build(input: KonataBuildInput) -> Self {
        let mut projector =
            KonataRelationProjector::new(&input.parents, &input.events, input.stream);
        projector.push(&input.relations);
        Self::build_projected(input, projector.finish())
    }

    #[must_use]
    pub fn build_projected(input: KonataBuildInput, projection: KonataRelationProjection) -> Self {
        futures::executor::block_on(Self::build_projected_inner(input, projection, false))
    }

    pub async fn build_cooperative(input: KonataBuildInput) -> Self {
        let mut projector =
            KonataRelationProjector::new(&input.parents, &input.events, input.stream);
        for relations in input.relations.chunks(4096) {
            projector.push(relations);
            crate::async_util::sleep_ms(0).await;
        }
        Self::build_projected_inner(input, projector.finish(), true).await
    }

    pub async fn build_projected_cooperative(
        input: KonataBuildInput,
        projection: KonataRelationProjection,
    ) -> Self {
        Self::build_projected_inner(input, projection, true).await
    }

    async fn build_projected_inner(
        input: KonataBuildInput,
        projection: KonataRelationProjection,
        cooperative: bool,
    ) -> Self {
        let mut records = KonataRecordProjector::new(
            input.parent_generator,
            input.event_generator,
            input.stream,
            input.parents.len(),
        );
        for transactions in input.parents.chunks(1024) {
            records.push(transactions);
            if cooperative {
                crate::async_util::sleep_ms(0).await;
            }
        }
        for transactions in input.events.chunks(1024) {
            records.push(transactions);
            if cooperative {
                crate::async_util::sleep_ms(0).await;
            }
        }
        Self::build_records_inner(records, projection, cooperative).await
    }

    async fn build_records_inner(
        records: KonataRecordProjector,
        projection: KonataRelationProjection,
        cooperative: bool,
    ) -> Self {
        let KonataRecordProjector {
            mut strings,
            thread_keys,
            mut rows,
            instruction_annotations,
            instruction_annotation_offsets,
            mut quality,
            pending_stages,
            pending_annotations,
            stage_names,
            lane_keys,
            ..
        } = records;
        mark_duplicate_rids(&mut rows, &mut quality);

        let tx_to_row_map = rows
            .tx_id
            .iter()
            .copied()
            .enumerate()
            .map(|(row, tx)| (tx, row))
            .collect::<HashMap<_, _>>();
        let mut per_row = vec![Vec::<(usize, KonataStage)>::new(); rows.len()];
        let mut annotations = Vec::new();
        let event_count = pending_stages.len();
        for (input_order, event) in pending_stages.iter().enumerate() {
            if cooperative && input_order.is_multiple_of(1024) {
                crate::async_util::sleep_ms(0).await;
            }
            let parents = projection.event_parents[input_order];
            let Some(row) = (parents.count > 0).then_some(parents.first_row as usize) else {
                quality.orphans += 1;
                continue;
            };

            let mut flags = StageFlags::default();
            if parents.count > 1 {
                flags.insert(StageFlags::MULTIPLE_PARENTS);
                quality.multiple_parents += 1;
            }
            if event.end < event.start {
                flags.insert(StageFlags::END_BEFORE_START);
                quality.end_before_start += 1;
            }
            if event.start < rows.begin[row] || event.end > rows.end[row] {
                flags.insert(StageFlags::OUT_OF_RANGE);
                quality.out_of_range += 1;
            }
            if event.unnamed {
                flags.insert(StageFlags::UNNAMED);
                quality.unnamed_stages += 1;
            }
            if event.unknown_lane {
                quality.unknown_lanes += 1;
            }
            let annotation_start = annotations.len() as u32;
            annotations.extend_from_slice(
                &pending_annotations
                    [event.annotation_start as usize..event.annotation_end as usize],
            );
            let annotation_end = annotations.len() as u32;

            per_row[row].push((
                input_order,
                KonataStage {
                    start: event.start,
                    end: event.end,
                    event_tx: event.event_tx,
                    name: event.name,
                    lane: event.lane,
                    flags,
                    detail_page: 0,
                    annotation_start,
                    annotation_end,
                },
            ));
        }

        let mut stages = Vec::with_capacity(event_count.saturating_sub(quality.orphans as usize));
        let mut stage_offsets = Vec::with_capacity(rows.len() + 1);
        stage_offsets.push(0);
        for (row, row_stages) in per_row.iter_mut().enumerate() {
            if cooperative && row.is_multiple_of(1024) {
                crate::async_util::sleep_ms(0).await;
            }
            row_stages.sort_by_key(|(input_order, stage)| (stage.start, stage.lane, *input_order));
            stages.extend(row_stages.drain(..).map(|(_, stage)| stage));
            stage_offsets.push(stages.len() as u32);
        }

        let mut tx_to_row = rows
            .tx_id
            .iter()
            .copied()
            .enumerate()
            .map(|(row, tx)| (tx, row as u32))
            .collect::<Vec<_>>();
        tx_to_row.sort_unstable();
        let mut event_to_row = stage_offsets
            .windows(2)
            .enumerate()
            .flat_map(|(row, offsets)| {
                stages[offsets[0] as usize..offsets[1] as usize]
                    .iter()
                    .map(move |stage| (stage.event_tx, row as u32))
            })
            .collect::<Vec<_>>();
        event_to_row.sort_unstable();
        let event_endpoints = stage_offsets
            .windows(2)
            .enumerate()
            .flat_map(|(row, offsets)| {
                stages[offsets[0] as usize..offsets[1] as usize]
                    .iter()
                    .map(move |stage| (stage.event_tx, (row as u32, stage.start)))
            })
            .collect::<HashMap<_, _>>();
        let mut dependency_name_ids = HashMap::<u32, u16>::new();
        let mut dependency_names = Vec::new();
        let mut dependencies = Vec::with_capacity(projection.dependencies.len());
        for (index, relation) in projection.dependencies.iter().enumerate() {
            if cooperative && index.is_multiple_of(1024) {
                crate::async_util::sleep_ms(0).await;
            }
            let source_id = relation.source_tx;
            let sink_id = relation.sink_tx;
            let Some(producer) = tx_to_row_map
                .get(&source_id)
                .map(|row| (*row as u32, None))
                .or_else(|| {
                    event_endpoints
                        .get(&source_id)
                        .map(|(row, tick)| (*row, Some(*tick)))
                })
            else {
                continue;
            };
            let Some(consumer) = tx_to_row_map
                .get(&sink_id)
                .map(|row| (*row as u32, None))
                .or_else(|| {
                    event_endpoints
                        .get(&sink_id)
                        .map(|(row, tick)| (*row, Some(*tick)))
                })
            else {
                continue;
            };
            let name_string = strings.intern(relation.name.clone());
            let next_name = u16::try_from(dependency_names.len()).unwrap_or(u16::MAX);
            let name = *dependency_name_ids.entry(name_string).or_insert_with(|| {
                dependency_names.push(name_string);
                next_name
            });
            dependencies.push((
                relation.input_order,
                KonataDependency {
                    producer_row: producer.0,
                    consumer_row: consumer.0,
                    name,
                    producer_tick: producer.1,
                    consumer_tick: consumer.1,
                },
            ));
        }
        dependencies.sort_by_key(|(input_order, edge)| {
            (
                edge.producer_row.max(edge.consumer_row),
                edge.producer_row.min(edge.consumer_row),
                *input_order,
            )
        });
        let dependencies = dependencies
            .into_iter()
            .map(|(_, dependency)| dependency)
            .collect::<Vec<_>>();
        let (producer_offsets, producer_edges) =
            dependency_csr(rows.len(), &dependencies, |dependency| {
                dependency.producer_row
            });
        let (consumer_offsets, consumer_edges) =
            dependency_csr(rows.len(), &dependencies, |dependency| {
                dependency.consumer_row
            });
        let mut sid_to_row = rows
            .sid
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, sid)| *sid != MISSING_U64)
            .map(|(row, sid)| (sid, row as u32))
            .collect::<Vec<_>>();
        sid_to_row.sort_unstable();
        let mut rid_to_row = rows
            .rid
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, rid)| *rid != MISSING_U64)
            .map(|(row, rid)| (rid, row as u32))
            .collect::<Vec<_>>();
        rid_to_row.sort_unstable();
        let mut thread_rid_to_row = rows
            .rid
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, rid)| *rid != MISSING_U64)
            .map(|(row, rid)| (rows.tid[row], rid, row as u32))
            .collect::<Vec<_>>();
        thread_rid_to_row.sort_unstable();
        let mut time_to_row = rows
            .begin
            .iter()
            .copied()
            .enumerate()
            .map(|(row, begin)| (begin, row as u32))
            .collect::<Vec<_>>();
        time_to_row.sort_unstable();
        let visibility = VisibilityIndex::from_flush_states(&rows.flushed);
        let all_ranges = BlockRangeIndex::build(rows.len(), |row| (rows.begin[row], rows.end[row]));
        let visible_ranges = BlockRangeIndex::build(visibility.visible_count(), |visible| {
            let row = visibility
                .select(visible)
                .expect("visible row is within the visibility index");
            (rows.begin[row], rows.end[row])
        });
        let flushed_count = rows
            .flushed
            .iter()
            .filter(|state| **state == FlushState::True)
            .count();

        let details = KonataDetailStore::encode(
            rows.len(),
            &stages,
            &stage_offsets,
            &annotations,
            &instruction_annotations,
            &instruction_annotation_offsets,
        )
        .expect("Konata detail pages only contain serializable projection records");

        Self {
            rows,
            strings: strings.values,
            stage_names,
            thread_names: thread_keys.names,
            lane_names: lane_keys.names,
            dependency_names,
            dependencies,
            quality,
            visibility,
            flushed_count,
            all_ranges,
            visible_ranges,
            tx_to_row,
            event_to_row,
            sid_to_row,
            rid_to_row,
            thread_rid_to_row,
            time_to_row,
            producer_offsets,
            producer_edges,
            consumer_offsets,
            consumer_edges,
            details,
        }
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn stage_count(&self) -> usize {
        self.details.stage_count()
    }

    #[must_use]
    pub fn detail_page_count(&self) -> usize {
        self.details.page_count()
    }

    /// Compatibility accessor for non-blocking UI detail. A cold page schedules
    /// background decoding and appears empty until it becomes resident. Renderers
    /// that need a visible fallback should use [`Self::try_stages_for_row`].
    #[must_use]
    pub fn stages_for_row(&self, row: usize) -> KonataRowDetail {
        self.details.row_detail(row, false)
    }

    /// Non-blocking detail accessor which distinguishes a cold or unavailable
    /// page from a row that genuinely has no stages.
    #[must_use]
    pub fn try_stages_for_row(&self, row: usize) -> Option<KonataRowDetail> {
        self.details.try_row_detail(row, false)
    }

    /// Detail accessor for search/statistics workers. This may decode a page
    /// synchronously and therefore must not be called from frame painting.
    #[must_use]
    pub fn stages_for_row_blocking(&self, row: usize) -> KonataRowDetail {
        self.details.row_detail(row, true)
    }

    #[must_use]
    pub fn detail_page(&self, page_id: u32) -> KonataDetailQuery {
        self.details.query(page_id)
    }

    pub fn prefetch_detail_rows(&self, rows: std::ops::Range<usize>) {
        self.details.prefetch_rows(rows);
    }

    #[must_use]
    pub fn detail_cache_telemetry(&self) -> KonataDetailCacheTelemetry {
        self.details.telemetry()
    }

    pub fn set_detail_cache_budget_bytes(&self, budget_bytes: usize) {
        self.details.set_budget(budget_bytes);
    }

    #[must_use]
    pub const fn resident_row_bytes() -> usize {
        5 * std::mem::size_of::<u64>()
            + 3 * std::mem::size_of::<u32>()
            + std::mem::size_of::<RowFlags>()
            + std::mem::size_of::<FlushState>()
    }

    #[must_use]
    pub fn detail_cache_idle(&self) -> bool {
        self.details.is_idle()
    }

    #[must_use]
    pub fn string(&self, id: u32) -> &str {
        self.strings.get(id as usize).map_or("", AsRef::as_ref)
    }

    #[must_use]
    pub fn stage_name(&self, stage: &KonataStage) -> &str {
        self.stage_names
            .get(stage.name as usize)
            .map_or("<unnamed stage>", |id| self.string(*id))
    }

    #[must_use]
    pub fn annotations_for_stage(&self, stage: &KonataStage) -> Vec<KonataAnnotation> {
        match self.details.query(stage.detail_page) {
            KonataDetailQuery::Ready(page) => page.annotations_for_stage(stage).to_vec(),
            KonataDetailQuery::Pending | KonataDetailQuery::Unavailable(_) => Vec::new(),
        }
    }

    /// Stage annotations for worker/table code. This can synchronously decode
    /// a cold detail page and must not be used while painting a frame.
    #[must_use]
    pub fn annotations_for_stage_blocking(&self, stage: &KonataStage) -> Vec<KonataAnnotation> {
        match self.details.get_blocking(stage.detail_page) {
            KonataDetailQuery::Ready(page) => page.annotations_for_stage(stage).to_vec(),
            KonataDetailQuery::Pending | KonataDetailQuery::Unavailable(_) => Vec::new(),
        }
    }

    /// Resolves a stable global stage ordinal without materializing a stage
    /// directory. Intended for lazy table workers, not frame painting.
    #[must_use]
    pub fn stage_by_ordinal_blocking(&self, ordinal: usize) -> Option<(usize, KonataStage)> {
        self.details.stage_by_ordinal(ordinal)
    }

    /// Resolves one event transaction to its parent row and paged stage.
    #[must_use]
    pub fn stage_for_event_blocking(&self, event_tx: u64) -> Option<(usize, KonataStage)> {
        let row = self.row_for_event(event_tx)?;
        self.stages_for_row_blocking(row)
            .iter()
            .find(|stage| stage.event_tx == event_tx)
            .cloned()
            .map(|stage| (row, stage))
    }

    #[must_use]
    pub fn annotations_for_row(&self, row: usize) -> Option<KonataAnnotationDetail> {
        self.details.row_annotations(row, false)
    }

    #[must_use]
    pub fn annotations_for_row_blocking(&self, row: usize) -> Option<KonataAnnotationDetail> {
        self.details.row_annotations(row, true)
    }

    #[must_use]
    pub fn row_annotation(&self, row: usize, names: &[&str]) -> Option<KonataAnnotation> {
        self.annotations_for_row_blocking(row)?
            .iter()
            .find(|annotation| {
                let name = self.string(annotation.name);
                names.contains(&name)
            })
            .cloned()
    }

    #[must_use]
    pub fn scalar_text(&self, value: &KonataScalar) -> String {
        match value {
            KonataScalar::Boolean(value) => value.to_string(),
            KonataScalar::Signed(value) => value.to_string(),
            KonataScalar::Unsigned(value) => value.to_string(),
            KonataScalar::Float(value) => value.to_string(),
            KonataScalar::Text(value) => self.string(*value).to_string(),
            KonataScalar::Time(value) => value.to_string(),
            KonataScalar::Error => "<invalid>".to_string(),
        }
    }

    #[must_use]
    pub fn annotation_text(&self, annotation: &KonataAnnotation) -> String {
        let value = self.scalar_text(&annotation.value);
        format!("{}: {value}", self.string(annotation.name))
    }

    #[must_use]
    pub fn thread_name(&self, tid: u32) -> &str {
        self.thread_names
            .get(tid as usize)
            .map_or("", |id| self.string(*id))
    }

    #[must_use]
    pub fn lane_name(&self, lane: u16) -> &str {
        self.lane_names
            .get(lane as usize)
            .map_or("", |id| self.string(*id))
    }

    #[must_use]
    pub fn dependency_name(&self, dependency: &KonataDependency) -> &str {
        self.dependency_names
            .get(dependency.name as usize)
            .map_or("relation", |id| self.string(*id))
    }

    #[must_use]
    pub fn dependencies_in_row_window(&self, start: usize, end: usize) -> &[KonataDependency] {
        let start = start as u32;
        let end = end as u32;
        let first = self.dependencies.partition_point(|dependency| {
            dependency.producer_row.max(dependency.consumer_row) < start
        });
        let last = self.dependencies.partition_point(|dependency| {
            dependency.producer_row.max(dependency.consumer_row) < end
        });
        &self.dependencies[first..last]
    }

    pub fn outgoing_dependencies(&self, row: usize) -> impl Iterator<Item = &KonataDependency> {
        dependency_slice(&self.producer_offsets, &self.producer_edges, row)
            .iter()
            .map(|edge| &self.dependencies[*edge as usize])
    }

    pub fn incoming_dependencies(&self, row: usize) -> impl Iterator<Item = &KonataDependency> {
        dependency_slice(&self.consumer_offsets, &self.consumer_edges, row)
            .iter()
            .map(|edge| &self.dependencies[*edge as usize])
    }

    #[must_use]
    pub fn producer_chain(&self, row: usize) -> KonataRowSet {
        self.producer_chain_cancellable(row, &AtomicBool::new(false))
            .unwrap_or_else(|| KonataRowSet::empty(self.row_count()))
    }

    #[must_use]
    pub fn producer_chain_cancellable(
        &self,
        row: usize,
        cancel: &AtomicBool,
    ) -> Option<KonataRowSet> {
        let mut result = KonataRowSet::empty(self.row_count());
        if row >= self.row_count() {
            return Some(result);
        }
        let mut pending = vec![row];
        let mut visited = 0usize;
        while let Some(consumer) = pending.pop() {
            if visited.is_multiple_of(1024) && cancel.load(Ordering::Relaxed) {
                return None;
            }
            if !result.insert(consumer) {
                continue;
            }
            visited += 1;
            pending.extend(
                self.incoming_dependencies(consumer)
                    .map(|dependency| dependency.producer_row as usize),
            );
        }
        Some(result)
    }

    /// Cooperative producer walk for UI-triggered analysis. It yields between
    /// bounded chunks on wasm and native runtimes alike.
    pub async fn producer_chain_cooperative(
        &self,
        row: usize,
        cancel: &AtomicBool,
    ) -> Option<KonataRowSet> {
        let mut result = KonataRowSet::empty(self.row_count());
        if row >= self.row_count() {
            return Some(result);
        }
        let mut pending = vec![row];
        let mut visited = 0usize;
        while let Some(consumer) = pending.pop() {
            if visited.is_multiple_of(1024) {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                crate::async_util::sleep_ms(0).await;
            }
            if !result.insert(consumer) {
                continue;
            }
            visited += 1;
            pending.extend(
                self.incoming_dependencies(consumer)
                    .map(|dependency| dependency.producer_row as usize),
            );
        }
        Some(result)
    }

    #[must_use]
    pub fn execution_tick(&self, row: usize) -> Option<u64> {
        self.stages_for_row(row)
            .iter()
            .find(|stage| matches!(self.stage_name(stage), "X" | "x" | "Ex" | "execute"))
            .map(|stage| stage.start)
    }

    #[must_use]
    pub fn search_text(&self, row: usize) -> String {
        let mut text = String::new();
        self.write_search_text(row, &mut text);
        text
    }

    pub fn write_search_text(&self, row: usize, text: &mut String) {
        use std::fmt::Write as _;

        text.clear();
        write!(text, "ID {row} tx {}", self.rows.tx_id[row]).ok();
        if let Some(sid) = self.rows.sid(row) {
            write!(text, " SID {sid}").ok();
        }
        if let Some(tid) = self.rows.tid(row) {
            write!(text, " TID {}", self.thread_name(tid)).ok();
        }
        if let Some(rid) = self.rows.rid(row) {
            write!(text, " RID {rid}").ok();
        }
        if let Some(label) = self.rows.label(row) {
            writeln!(text, "\n{}", self.string(label)).ok();
        }
        if let Some(detail) = self.rows.detail(row) {
            writeln!(text, "\n{}", self.string(detail)).ok();
        }
        for stage in self.stages_for_row_blocking(row).iter() {
            write!(text, "\n{}", self.stage_name(stage)).ok();
            for annotation in self.annotations_for_stage(stage) {
                write!(text, "\n{}", self.annotation_text(&annotation)).ok();
            }
        }
    }

    #[must_use]
    pub fn row_for_transaction(&self, tx_id: u64) -> Option<usize> {
        self.tx_to_row
            .binary_search_by_key(&tx_id, |(id, _)| *id)
            .ok()
            .map(|index| self.tx_to_row[index].1 as usize)
    }

    #[must_use]
    pub fn row_for_event(&self, event_tx: u64) -> Option<usize> {
        self.event_to_row
            .binary_search_by_key(&event_tx, |(id, _)| *id)
            .ok()
            .map(|index| self.event_to_row[index].1 as usize)
    }

    #[must_use]
    pub fn row_for_sid(&self, sid: u64) -> Option<usize> {
        unique_index_lookup(&self.sid_to_row, sid)
    }

    #[must_use]
    pub fn row_for_rid(&self, rid: u64) -> Option<usize> {
        unique_index_lookup(&self.rid_to_row, rid)
    }

    #[must_use]
    pub fn row_for_thread_rid(&self, thread: Option<&str>, rid: u64) -> Option<usize> {
        let tid = match thread {
            Some(thread) => self
                .thread_names
                .iter()
                .enumerate()
                .find(|(tid, _)| self.thread_name(*tid as u32) == thread)
                .map(|(tid, _)| tid as u32)?,
            None => MISSING_U32,
        };
        let first = self
            .thread_rid_to_row
            .partition_point(|(candidate_tid, candidate_rid, _)| {
                (*candidate_tid, *candidate_rid) < (tid, rid)
            });
        let entry =
            self.thread_rid_to_row
                .get(first)
                .filter(|(candidate_tid, candidate_rid, _)| {
                    (*candidate_tid, *candidate_rid) == (tid, rid)
                })?;
        let unique = self.thread_rid_to_row.get(first + 1).is_none_or(
            |(candidate_tid, candidate_rid, _)| (*candidate_tid, *candidate_rid) != (tid, rid),
        );
        unique.then_some(entry.2 as usize)
    }

    #[must_use]
    pub fn nearest_row_for_tick(&self, tick: u64) -> Option<usize> {
        let position = self
            .time_to_row
            .partition_point(|(candidate, _)| *candidate < tick);
        match (position.checked_sub(1), self.time_to_row.get(position)) {
            (None, None) => None,
            (Some(previous), None) => Some(self.time_to_row[previous].1 as usize),
            (None, Some((_, row))) => Some(*row as usize),
            (Some(previous), Some((next_tick, next_row))) => {
                let (previous_tick, previous_row) = self.time_to_row[previous];
                if tick.saturating_sub(previous_tick) <= next_tick.saturating_sub(tick) {
                    Some(previous_row as usize)
                } else {
                    Some(*next_row as usize)
                }
            }
        }
    }

    /// Exact fetch/retire envelope and counts for a logical row range.
    /// Query work is bounded by two partial 64-row blocks plus a logarithmic
    /// tree query, independent of the represented trace span.
    #[must_use]
    pub fn range_extent(
        &self,
        start: usize,
        end: usize,
        hide_flushed: bool,
    ) -> Option<(u64, u64, usize, usize)> {
        let logical_count = if hide_flushed {
            self.visibility.visible_count()
        } else {
            self.row_count()
        };
        let start = start.min(logical_count);
        let end = end.min(logical_count);
        let index = if hide_flushed {
            &self.visible_ranges
        } else {
            &self.all_ranges
        };
        let (min_begin, max_end) = index.query(start, end, |logical| {
            let row = if hide_flushed {
                self.visibility
                    .select(logical)
                    .expect("logical row is visible")
            } else {
                logical
            };
            (self.rows.begin[row], self.rows.end[row])
        })?;
        let count = end - start;
        let flushed = if hide_flushed {
            0
        } else {
            count - (self.visibility.rank(end) - self.visibility.rank(start))
        };
        Some((min_begin, max_end, flushed, count))
    }
}

fn unique_index_lookup(index: &[(u64, u32)], key: u64) -> Option<usize> {
    let first = index.partition_point(|(candidate, _)| *candidate < key);
    let entry = index
        .get(first)
        .filter(|(candidate, _)| *candidate == key)?;
    let unique = index
        .get(first + 1)
        .is_none_or(|(candidate, _)| *candidate != key);
    unique.then_some(entry.1 as usize)
}

fn dependency_csr(
    row_count: usize,
    dependencies: &[KonataDependency],
    row_of: impl Fn(&KonataDependency) -> u32,
) -> (Vec<u32>, Vec<u32>) {
    let mut offsets = vec![0u32; row_count + 1];
    for dependency in dependencies {
        offsets[row_of(dependency) as usize + 1] += 1;
    }
    for row in 0..row_count {
        offsets[row + 1] += offsets[row];
    }
    let mut edges = vec![0u32; dependencies.len()];
    let mut tails = offsets[..row_count].to_vec();
    for (edge, dependency) in dependencies.iter().enumerate() {
        let row = row_of(dependency) as usize;
        edges[tails[row] as usize] = edge as u32;
        tails[row] += 1;
    }
    (offsets, edges)
}

fn dependency_slice<'a>(offsets: &[u32], edges: &'a [u32], row: usize) -> &'a [u32] {
    offsets.get(row..=row + 1).map_or(&[], |offsets| {
        &edges[offsets[0] as usize..offsets[1] as usize]
    })
}

const RANGE_BLOCK_ROWS: usize = 64;

#[derive(Debug, Clone)]
struct BlockRangeIndex {
    block_count: usize,
    tree_size: usize,
    min_tree: Vec<u64>,
    max_tree: Vec<u64>,
}

impl BlockRangeIndex {
    fn build(length: usize, mut value: impl FnMut(usize) -> (u64, u64)) -> Self {
        let block_count = length.div_ceil(RANGE_BLOCK_ROWS);
        let tree_size = block_count.max(1).next_power_of_two();
        let mut min_tree = vec![u64::MAX; tree_size * 2];
        let mut max_tree = vec![0; tree_size * 2];
        for block in 0..block_count {
            let start = block * RANGE_BLOCK_ROWS;
            let end = (start + RANGE_BLOCK_ROWS).min(length);
            let (min_begin, max_end) = (start..end)
                .map(&mut value)
                .fold((u64::MAX, 0), |(min_begin, max_end), (begin, end)| {
                    (min_begin.min(begin), max_end.max(end))
                });
            min_tree[tree_size + block] = min_begin;
            max_tree[tree_size + block] = max_end;
        }
        for node in (1..tree_size).rev() {
            min_tree[node] = min_tree[node * 2].min(min_tree[node * 2 + 1]);
            max_tree[node] = max_tree[node * 2].max(max_tree[node * 2 + 1]);
        }
        Self {
            block_count,
            tree_size,
            min_tree,
            max_tree,
        }
    }

    fn query(
        &self,
        start: usize,
        end: usize,
        mut value: impl FnMut(usize) -> (u64, u64),
    ) -> Option<(u64, u64)> {
        if start >= end {
            return None;
        }
        let first_full = start.div_ceil(RANGE_BLOCK_ROWS);
        let last_full = end / RANGE_BLOCK_ROWS;
        let leading_end = end.min(first_full * RANGE_BLOCK_ROWS);
        let trailing_start = start.max(last_full * RANGE_BLOCK_ROWS);
        let mut result = (u64::MAX, 0);

        for logical in start..leading_end {
            merge_range(&mut result, value(logical));
        }
        if first_full < last_full {
            merge_range(&mut result, self.query_blocks(first_full, last_full));
        }
        for logical in trailing_start.max(leading_end)..end {
            merge_range(&mut result, value(logical));
        }
        (result.0 != u64::MAX).then_some(result)
    }

    fn query_blocks(&self, start: usize, end: usize) -> (u64, u64) {
        debug_assert!(start < end && end <= self.block_count);
        let mut left = start + self.tree_size;
        let mut right = end + self.tree_size;
        let mut result = (u64::MAX, 0);
        while left < right {
            if left % 2 == 1 {
                merge_range(&mut result, (self.min_tree[left], self.max_tree[left]));
                left += 1;
            }
            if right % 2 == 1 {
                right -= 1;
                merge_range(&mut result, (self.min_tree[right], self.max_tree[right]));
            }
            left /= 2;
            right /= 2;
        }
        result
    }
}

fn merge_range(target: &mut (u64, u64), value: (u64, u64)) {
    target.0 = target.0.min(value.0);
    target.1 = target.1.max(value.1);
}

#[derive(Debug, Clone, Default)]
struct StringInterner {
    values: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, u32>,
}

impl StringInterner {
    fn intern(&mut self, value: Arc<str>) -> u32 {
        if let Some(id) = self.ids.get(&value) {
            return *id;
        }
        let id = self.values.len() as u32;
        self.values.push(value.clone());
        self.ids.insert(value, id);
        id
    }

    fn intern_data(&mut self, value: &DataType) -> u32 {
        match value {
            DataType::Enumeration(value)
            | DataType::BitVector(value)
            | DataType::LogicVector(value)
            | DataType::String(value) => self.intern(value.clone()),
            _ => self.intern(Arc::from(format_data(value))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TypedKey {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Text(Arc<str>),
}

impl TypedKey {
    fn from_data(value: &DataType) -> Option<Self> {
        match value {
            DataType::Boolean(value) => Some(Self::Boolean(*value)),
            DataType::Integer(value) => Some(Self::Signed(*value)),
            DataType::Unsigned(value) | DataType::Pointer(value) | DataType::Time(value) => {
                Some(Self::Unsigned(*value))
            }
            DataType::Enumeration(value)
            | DataType::BitVector(value)
            | DataType::LogicVector(value)
            | DataType::String(value) => Some(Self::Text(value.clone())),
            DataType::FloatingPointNumber(_)
            | DataType::FixedPointInteger(_)
            | DataType::UnsignedFixedPointInteger(_)
            | DataType::Error => None,
        }
    }

    fn display(&self) -> Arc<str> {
        match self {
            Self::Boolean(value) => Arc::from(value.to_string()),
            Self::Signed(value) => Arc::from(value.to_string()),
            Self::Unsigned(value) => Arc::from(value.to_string()),
            Self::Text(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct KeyInterner {
    ids: HashMap<TypedKey, u32>,
    names: Vec<u32>,
}

impl KeyInterner {
    fn intern(&mut self, value: TypedKey, strings: &mut StringInterner) -> u32 {
        if let Some(id) = self.ids.get(&value) {
            return *id;
        }
        let id = self.names.len() as u32;
        self.names.push(strings.intern(value.display()));
        self.ids.insert(value, id);
        id
    }
}

fn attribute<'a>(tx: &'a Transaction, name: &str) -> Option<&'a DataType> {
    tx.attributes
        .iter()
        .find(|attribute| attribute.name.as_ref() == name)
        .map(|attribute| &attribute.data_type)
}

fn is_projected_instruction_attribute(name: &str) -> bool {
    matches!(
        name,
        "label" | "detail" | "insn_id_in_sim" | "thread_id" | "retire_id" | "flushed"
    )
}

fn event_name_attribute(tx: &Transaction) -> Option<&Attribute> {
    tx.attributes
        .iter()
        .find(|attribute| {
            matches!(attribute.kind, AttributeType::BEGIN)
                && attribute.name.as_ref() == EVENT_NAME_ATTRIBUTE
        })
        .or_else(|| {
            tx.attributes
                .iter()
                .find(|attribute| attribute.name.as_ref() == EVENT_NAME_ATTRIBUTE)
        })
}

fn unsigned_value(value: &DataType) -> Option<u64> {
    match value {
        DataType::Unsigned(value) | DataType::Pointer(value) | DataType::Time(value) => {
            Some(*value)
        }
        DataType::Integer(value) => u64::try_from(*value).ok(),
        _ => None,
    }
}

fn scalar_from_data(value: &DataType, strings: &mut StringInterner) -> KonataScalar {
    match value {
        DataType::Boolean(value) => KonataScalar::Boolean(*value),
        DataType::Integer(value) => KonataScalar::Signed(*value),
        DataType::Unsigned(value) | DataType::Pointer(value) => KonataScalar::Unsigned(*value),
        DataType::FloatingPointNumber(value)
        | DataType::FixedPointInteger(value)
        | DataType::UnsignedFixedPointInteger(value) => KonataScalar::Float(*value),
        DataType::Enumeration(value)
        | DataType::BitVector(value)
        | DataType::LogicVector(value)
        | DataType::String(value) => KonataScalar::Text(strings.intern(value.clone())),
        DataType::Time(value) => KonataScalar::Time(*value),
        DataType::Error => KonataScalar::Error,
    }
}

fn format_data(value: &DataType) -> String {
    match value {
        DataType::Boolean(value) => value.to_string(),
        DataType::Enumeration(value)
        | DataType::BitVector(value)
        | DataType::LogicVector(value)
        | DataType::String(value) => value.to_string(),
        DataType::Integer(value) => value.to_string(),
        DataType::Unsigned(value) | DataType::Pointer(value) | DataType::Time(value) => {
            value.to_string()
        }
        DataType::FloatingPointNumber(value)
        | DataType::FixedPointInteger(value)
        | DataType::UnsignedFixedPointInteger(value) => value.to_string(),
        DataType::Error => String::new(),
    }
}

fn mark_duplicate_rids(rows: &mut InstructionColumns, quality: &mut KonataQualityCounters) {
    let mut first = HashMap::<(u32, u64), usize>::new();
    for row in 0..rows.len() {
        let rid = rows.rid[row];
        if rid == MISSING_U64 {
            continue;
        }
        let key = (rows.tid[row], rid);
        if let Some(previous) = first.insert(key, row) {
            if !rows.flags[previous].contains(RowFlags::DUPLICATE_RID) {
                rows.flags[previous].insert(RowFlags::DUPLICATE_RID);
                quality.duplicate_rid += 1;
            }
            rows.flags[row].insert(RowFlags::DUPLICATE_RID);
            quality.duplicate_rid += 1;
        }
    }
}

/// Rank/select bitvector containing every row that is not explicitly flushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityIndex {
    words: Vec<u64>,
    ranks: Vec<u32>,
    visible: usize,
    rows: usize,
}

impl VisibilityIndex {
    fn from_flush_states(states: &[FlushState]) -> Self {
        let mut words = vec![0u64; states.len().div_ceil(64)];
        for (row, state) in states.iter().enumerate() {
            if *state != FlushState::True {
                words[row / 64] |= 1 << (row % 64);
            }
        }
        let mut running = 0u32;
        let ranks = words
            .iter()
            .map(|word| {
                let before = running;
                running += word.count_ones();
                before
            })
            .collect();
        Self {
            words,
            ranks,
            visible: running as usize,
            rows: states.len(),
        }
    }

    #[must_use]
    pub fn visible_count(&self) -> usize {
        self.visible
    }

    #[must_use]
    pub fn is_visible(&self, row: usize) -> bool {
        row < self.rows && self.words[row / 64] & (1 << (row % 64)) != 0
    }

    /// Number of visible rows strictly before `row`.
    #[must_use]
    pub fn rank(&self, row: usize) -> usize {
        let row = row.min(self.rows);
        let word = row / 64;
        if word == self.words.len() {
            return self.visible;
        }
        let bit = row % 64;
        let prefix = if bit == 0 { 0 } else { (1u64 << bit) - 1 };
        self.ranks[word] as usize + (self.words[word] & prefix).count_ones() as usize
    }

    #[must_use]
    pub fn select(&self, visible_row: usize) -> Option<usize> {
        if visible_row >= self.visible {
            return None;
        }
        let word_index = self
            .ranks
            .partition_point(|rank| *rank as usize <= visible_row)
            .saturating_sub(1);
        let mut word = self.words[word_index];
        let mut remaining = visible_row - self.ranks[word_index] as usize;
        while remaining > 0 {
            word &= word - 1;
            remaining -= 1;
        }
        Some(word_index * 64 + word.trailing_zeros() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction_events::test_util::{ftr_from_json, generator, stream, tx};
    use ftr_parser::parse::parse_ftr;
    use project_root::get_project_root;
    use serde_json::json;

    #[test]
    fn visibility_rank_select_round_trip() {
        let states = [
            FlushState::False,
            FlushState::True,
            FlushState::Unknown,
            FlushState::True,
            FlushState::False,
        ];
        let index = VisibilityIndex::from_flush_states(&states);
        assert_eq!(index.visible_count(), 3);
        assert_eq!(
            (0..3).map(|row| index.select(row)).collect::<Vec<_>>(),
            vec![Some(0), Some(2), Some(4)]
        );
        assert_eq!(
            (0..=5).map(|row| index.rank(row)).collect::<Vec<_>>(),
            vec![0, 1, 1, 2, 2, 3]
        );
    }

    #[test]
    fn visibility_handles_word_boundaries() {
        let mut states = vec![FlushState::True; 130];
        for row in [0, 63, 64, 65, 129] {
            states[row] = FlushState::False;
        }
        let index = VisibilityIndex::from_flush_states(&states);
        assert_eq!(index.visible_count(), 5);
        assert_eq!(
            (0..5)
                .filter_map(|row| index.select(row))
                .collect::<Vec<_>>(),
            vec![0, 63, 64, 65, 129]
        );
        assert_eq!(index.rank(64), 2);
        assert_eq!(index.rank(130), 5);
    }

    #[test]
    fn search_hits_rank_select_wraps_in_both_directions() {
        let mut hits = KonataSearchHits::empty(130);
        for row in [0, 63, 64, 129] {
            hits.insert(row);
        }
        hits.finish();

        assert_eq!(hits.count(), 4);
        assert_eq!(hits.next(0, false), Some(63));
        assert_eq!(hits.next(63, false), Some(64));
        assert_eq!(hits.next(129, false), Some(0));
        assert_eq!(hits.next(0, true), Some(129));
        assert_eq!(hits.next(64, true), Some(63));
        assert!(hits.contains(129));
        assert!(!hits.contains(128));
    }

    #[test]
    fn block_range_queries_match_brute_force_across_boundaries() {
        let values = (0..257u64)
            .map(|row| ((row * 37) % 101, 500 - (row * 19) % 211))
            .collect::<Vec<_>>();
        let index = BlockRangeIndex::build(values.len(), |row| values[row]);
        for start in [0, 1, 31, 63, 64, 65, 127, 129, 255] {
            for end in [start + 1, (start + 17).min(values.len()), values.len()] {
                let expected = values[start..end]
                    .iter()
                    .copied()
                    .fold((u64::MAX, 0u64), |(min_begin, max_end), (begin, end)| {
                        (min_begin.min(begin), max_end.max(end))
                    });
                assert_eq!(index.query(start, end, |row| values[row]), Some(expected));
            }
        }
    }

    #[test]
    fn randomized_rank_select_and_envelopes_match_naive_models() {
        let mut random = 0x6a09_e667_f3bc_c909u64;
        let mut next = || {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random
        };

        for len in [1, 2, 63, 64, 65, 257, 1_033] {
            let states = (0..len)
                .map(|_| match next() % 3 {
                    0 => FlushState::True,
                    1 => FlushState::False,
                    _ => FlushState::Unknown,
                })
                .collect::<Vec<_>>();
            let visible = states
                .iter()
                .enumerate()
                .filter_map(|(row, state)| (*state != FlushState::True).then_some(row))
                .collect::<Vec<_>>();
            let visibility = VisibilityIndex::from_flush_states(&states);
            assert_eq!(visibility.visible_count(), visible.len());
            for row in 0..=len {
                assert_eq!(
                    visibility.rank(row),
                    visible.partition_point(|candidate| *candidate < row)
                );
            }
            assert_eq!(
                (0..visible.len())
                    .filter_map(|rank| visibility.select(rank))
                    .collect::<Vec<_>>(),
                visible
            );

            let values = (0..len)
                .map(|row| {
                    let begin = next() % 10_000;
                    (begin, begin.saturating_add((next() + row as u64) % 500))
                })
                .collect::<Vec<_>>();
            let ranges = BlockRangeIndex::build(len, |row| values[row]);
            for _ in 0..2_000 {
                let left = next() as usize % len;
                let right = next() as usize % len;
                let start = left.min(right);
                let end = left.max(right) + 1;
                let expected = values[start..end]
                    .iter()
                    .copied()
                    .fold((u64::MAX, 0u64), |(min_begin, max_end), (begin, end)| {
                        (min_begin.min(begin), max_end.max(end))
                    });
                assert_eq!(ranges.query(start, end, |row| values[row]), Some(expected));
            }
        }
    }

    #[test]
    fn checked_in_konata_trace_builds_complete_projection() {
        let mut ftr = parse_ftr(
            get_project_root()
                .expect("project root")
                .join("examples/kanata-sample-2.ftr"),
        )
        .expect("parse sample header");
        ftr.load_stream_into_memory(StreamId(1))
            .expect("load sample stream");
        let input = KonataBuildInput {
            parent_generator: GeneratorId(10),
            event_generator: GeneratorId(11),
            stream: StreamId(1),
            parents: ftr
                .get_generator(GeneratorId(10))
                .expect("instruction generator")
                .transactions
                .clone(),
            events: ftr
                .get_generator(GeneratorId(11))
                .expect("stage generator")
                .transactions
                .clone(),
            relations: ftr.tx_relations.clone(),
        };
        let mut projector =
            KonataRelationProjector::new(&input.parents, &input.events, input.stream);
        for relations in input.relations.chunks(997) {
            projector.push(relations);
        }
        let projected = KonataModel::build_projected(input.clone(), projector.finish());
        let model = KonataModel::build(input);

        assert_eq!(model.row_count(), 4_041);
        assert_eq!(model.stage_count(), 51_961);
        assert_eq!(projected.row_count(), model.row_count());
        assert_eq!(projected.stage_count(), model.stage_count());
        assert_eq!(projected.quality, model.quality);
        assert_eq!(projected.dependencies, model.dependencies);
        assert_eq!(projected.strings, model.strings);
        assert!(KonataModel::resident_row_bytes() <= 64);
        let storage = model.detail_cache_telemetry();
        assert_eq!(storage.page_count, 1);
        assert!(
            storage.encoded_bytes
                < model.stage_count() * 24 + model.row_count() * 2 * std::mem::size_of::<u32>()
        );
        assert_eq!(model.quality.orphans, 0);
        assert_eq!(model.quality.begin_regressions, 76);
        assert_eq!(model.quality.missing_rid, 41);
        assert_eq!(model.quality.duplicate_rid, 0);
        assert_eq!(model.detail_page_count(), 1);
        assert_eq!(model.row_for_transaction(1), Some(3));
        assert_eq!(model.row_for_sid(8), Some(0));
        assert_eq!(model.row_for_rid(0), Some(3));
        let thread = model.rows.tid(3).map(|tid| model.thread_name(tid));
        assert_eq!(model.row_for_thread_rid(thread, 0), Some(3));
        assert_eq!(model.nearest_row_for_tick(0), Some(0));
        let first_event = model.stages_for_row_blocking(0)[0].event_tx;
        assert_eq!(model.row_for_event(first_event), Some(0));
    }

    #[test]
    #[ignore = "multi-million-row end-to-end projection acceptance"]
    fn multi_million_projected_rows_keep_model_and_detail_queries_bounded() {
        use ftr_parser::types::{Event, TransactionId};

        const ROWS: usize = 2_000_000;
        let transaction = |row: usize, generator: GeneratorId, event: bool| Transaction {
            event: Event {
                tx_id: TransactionId(if event {
                    ROWS as u64 + row as u64
                } else {
                    row as u64
                }),
                gen_id: generator,
                start_time: row as u64 * 2,
                end_time: row as u64 * 2 + 1,
            },
            attributes: Vec::new(),
            inc_relations: Vec::new(),
            out_relations: Vec::new(),
            row: 0,
        };
        let started = std::time::Instant::now();
        let mut records =
            KonataRecordProjector::new(GeneratorId(10), GeneratorId(11), StreamId(1), ROWS);
        for start in (0..ROWS).step_by(4096) {
            let end = (start + 4096).min(ROWS);
            let batch = (start..end)
                .map(|row| transaction(row, GeneratorId(10), false))
                .chain((start..end).map(|row| transaction(row, GeneratorId(11), true)))
                .collect::<Vec<_>>();
            records.push(&batch);
        }
        let mut relation_projector = records.relation_projector();
        let parent_of = Arc::<str>::from(EVENT_PARENT_RELATION);
        for start in (0..ROWS).step_by(4096) {
            let end = (start + 4096).min(ROWS);
            let relations = (start..end)
                .map(|row| TxRelation {
                    name: parent_of.clone(),
                    source_tx_id: TransactionId(row as u64),
                    sink_tx_id: TransactionId(ROWS as u64 + row as u64),
                    source_stream_id: StreamId(1),
                    sink_stream_id: StreamId(1),
                })
                .collect::<Vec<_>>();
            relation_projector.push(&relations);
        }
        let projection = relation_projector.finish();
        let model = futures::executor::block_on(records.finish(projection, false));
        assert_eq!(model.row_count(), ROWS);
        assert_eq!(model.stage_count(), ROWS);
        assert!(KonataModel::resident_row_bytes() <= 64);
        assert_eq!(model.row_for_transaction((ROWS - 1) as u64), Some(ROWS - 1));
        assert_eq!(model.row_for_event((ROWS * 2 - 1) as u64), Some(ROWS - 1));
        assert_eq!(
            model.range_extent(0, ROWS, false),
            Some((0, ROWS as u64 * 2 - 1, 0, ROWS))
        );

        let first = model.stages_for_row_blocking(0);
        assert_eq!(first.len(), 1);
        let first_page_bytes = model.detail_cache_telemetry().resident_bytes;
        model.set_detail_cache_budget_bytes(first_page_bytes);
        assert_eq!(model.stages_for_row_blocking(ROWS / 2).len(), 1);
        assert_eq!(model.stages_for_row_blocking(ROWS - 1).len(), 1);
        let telemetry = model.detail_cache_telemetry();
        assert!(telemetry.resident_bytes <= telemetry.budget_bytes);
        assert!(telemetry.evictions >= 2);
        eprintln!(
            "Konata full projection acceptance: {ROWS} rows/stages, {} pages, {} encoded MiB, build {:?}, decoded budget {} KiB",
            telemetry.page_count,
            telemetry.encoded_bytes / (1024 * 1024),
            started.elapsed(),
            telemetry.budget_bytes / 1024,
        );
    }

    #[test]
    fn malformed_pipeline_data_is_preserved_and_counted() {
        let mut invalid = tx(5, 11, 160, 150, None, &[(1, 1)]);
        invalid["attributes"]
            .as_array_mut()
            .expect("attribute array")
            .push(json!({
                "kind": "RECORD",
                "name": "lane",
                "data_type": {"FloatingPointNumber": 1.5},
            }));
        let mut second_parent = tx(3, 10, 50, 180, None, &[]);
        second_parent["inc_relations"]
            .as_array_mut()
            .expect("relation array")
            .push(json!({
                "name": "wakeup",
                "source_tx_id": 1,
                "sink_tx_id": 3,
                "source_stream_id": 1,
                "sink_stream_id": 1,
            }));
        let ftr = ftr_from_json(
            json!({"1": stream(1, "cpu", &[10, 11])}),
            json!({
                "10": generator(10, 1, "instruction", json!([
                    tx(1, 10, 100, 200, None, &[]),
                    second_parent,
                ])),
                "11": generator(11, 1, "instruction.events", json!([
                    tx(2, 11, 90, 210, Some("X"), &[(1, 1), (3, 1)]),
                    tx(4, 11, 120, 120, Some("orphan"), &[]),
                    invalid,
                ])),
            }),
        );
        let model = KonataModel::build(KonataBuildInput {
            parent_generator: GeneratorId(10),
            event_generator: GeneratorId(11),
            stream: StreamId(1),
            parents: ftr
                .get_generator(GeneratorId(10))
                .unwrap()
                .transactions
                .clone(),
            events: ftr
                .get_generator(GeneratorId(11))
                .unwrap()
                .transactions
                .clone(),
            relations: ftr.tx_relations.clone(),
        });

        assert_eq!(model.quality.orphans, 1);
        assert_eq!(model.quality.multiple_parents, 1);
        assert_eq!(model.quality.unnamed_stages, 1);
        assert_eq!(model.quality.out_of_range, 1);
        assert_eq!(model.quality.end_before_start, 1);
        assert_eq!(model.quality.begin_regressions, 1);
        assert_eq!(model.quality.unknown_lanes, 1);
        let stages = model.stages_for_row_blocking(0);
        assert_eq!(stages.len(), 2);
        assert_eq!((stages[0].start, stages[0].end), (90, 210));
        assert!(stages[0].flags.contains(StageFlags::OUT_OF_RANGE));
        assert!(stages[0].flags.contains(StageFlags::MULTIPLE_PARENTS));
        assert_eq!((stages[1].start, stages[1].end), (160, 150));
        assert!(stages[1].flags.contains(StageFlags::END_BEFORE_START));
        assert!(stages[1].flags.contains(StageFlags::UNNAMED));
        assert_eq!(model.lane_name(stages[1].lane), "<unknown lane 1.5>");
        assert!(
            model
                .annotations_for_stage(&stages[1])
                .iter()
                .any(|annotation| model.string(annotation.name) == "lane"
                    && annotation.value == KonataScalar::Float(1.5))
        );
        assert_eq!(model.dependencies.len(), 1);
        assert_eq!(model.dependency_name(&model.dependencies[0]), "wakeup");
        assert_eq!(
            (
                model.dependencies[0].producer_row,
                model.dependencies[0].consumer_row
            ),
            (0, 1)
        );
        assert_eq!(model.outgoing_dependencies(0).count(), 1);
        assert_eq!(model.incoming_dependencies(1).count(), 1);
        assert_eq!(model.incoming_dependencies(0).count(), 0);
        let chain = model.producer_chain(1);
        assert_eq!(chain.count(), 2);
        assert!(chain.contains(0));
        assert!(chain.contains(1));
        let cancelled = AtomicBool::new(true);
        assert!(model.producer_chain_cancellable(1, &cancelled).is_none());
    }
}
