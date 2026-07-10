use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, Range},
    sync::{Arc, LazyLock, Mutex, atomic::Ordering},
};

use serde::{Deserialize, Serialize};

use crate::{EGUI_CONTEXT, OUTSTANDING_TRANSACTIONS};

use super::{KonataAnnotation, KonataStage};

pub const KONATA_DETAIL_PAGE_ROWS: usize = 4_096;

#[cfg(target_arch = "wasm32")]
const DEFAULT_DETAIL_CACHE_BYTES: usize = 64 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_DETAIL_CACHE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KonataDetailCacheTelemetry {
    pub budget_bytes: usize,
    pub encoded_bytes: usize,
    pub page_count: usize,
    pub resident_bytes: usize,
    pub resident_pages: usize,
    pub pending_pages: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub decode_errors: u64,
}

#[derive(Debug, Clone)]
pub enum KonataDetailQuery {
    Ready(Arc<KonataDetailPage>),
    Pending,
    Unavailable(Arc<str>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KonataDetailPage {
    pub id: u32,
    pub row_start: usize,
    pub row_end: usize,
    row_stage_offsets: Vec<u32>,
    stages: Vec<KonataStage>,
    row_annotation_offsets: Vec<u32>,
    row_annotations: Vec<KonataAnnotation>,
    stage_annotations: Vec<KonataAnnotation>,
}

impl KonataDetailPage {
    #[must_use]
    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    #[must_use]
    pub fn decoded_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.row_stage_offsets.capacity() * std::mem::size_of::<u32>()
            + self.stages.capacity() * std::mem::size_of::<KonataStage>()
            + self.row_annotation_offsets.capacity() * std::mem::size_of::<u32>()
            + self.row_annotations.capacity() * std::mem::size_of::<KonataAnnotation>()
            + self.stage_annotations.capacity() * std::mem::size_of::<KonataAnnotation>()
    }

    fn row_index(&self, row: usize) -> Option<usize> {
        (self.row_start..self.row_end)
            .contains(&row)
            .then_some(row - self.row_start)
    }

    fn stages_range(&self, row: usize) -> Option<Range<usize>> {
        let row = self.row_index(row)?;
        Some(self.row_stage_offsets[row] as usize..self.row_stage_offsets[row + 1] as usize)
    }

    fn row_annotations_range(&self, row: usize) -> Option<Range<usize>> {
        let row = self.row_index(row)?;
        Some(
            self.row_annotation_offsets[row] as usize
                ..self.row_annotation_offsets[row + 1] as usize,
        )
    }

    fn stage_annotations_range(&self, stage: &KonataStage) -> Range<usize> {
        stage.annotation_start as usize..stage.annotation_end as usize
    }

    pub(crate) fn annotations_for_stage(&self, stage: &KonataStage) -> &[KonataAnnotation] {
        &self.stage_annotations[self.stage_annotations_range(stage)]
    }
}

#[derive(Debug, Clone)]
pub struct KonataRowDetail {
    page: Arc<KonataDetailPage>,
    stages: Range<usize>,
}

impl KonataRowDetail {
    fn new(page: Arc<KonataDetailPage>, row: usize) -> Option<Self> {
        let stages = page.stages_range(row)?;
        Some(Self { page, stages })
    }

    #[must_use]
    pub fn annotations_for_stage(&self, stage: &KonataStage) -> &[KonataAnnotation] {
        self.page.annotations_for_stage(stage)
    }

    fn empty() -> Self {
        static EMPTY_PAGE: LazyLock<Arc<KonataDetailPage>> = LazyLock::new(|| {
            Arc::new(KonataDetailPage {
                id: u32::MAX,
                row_start: 0,
                row_end: 0,
                row_stage_offsets: vec![0],
                stages: Vec::new(),
                row_annotation_offsets: vec![0],
                row_annotations: Vec::new(),
                stage_annotations: Vec::new(),
            })
        });
        Self {
            page: EMPTY_PAGE.clone(),
            stages: 0..0,
        }
    }
}

impl Deref for KonataRowDetail {
    type Target = [KonataStage];

    fn deref(&self) -> &Self::Target {
        &self.page.stages[self.stages.clone()]
    }
}

impl<'a> IntoIterator for &'a KonataRowDetail {
    type Item = &'a KonataStage;
    type IntoIter = std::slice::Iter<'a, KonataStage>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone)]
pub struct KonataAnnotationDetail {
    page: Arc<KonataDetailPage>,
    annotations: Range<usize>,
}

impl Deref for KonataAnnotationDetail {
    type Target = [KonataAnnotation];

    fn deref(&self) -> &Self::Target {
        &self.page.row_annotations[self.annotations.clone()]
    }
}

impl<'a> IntoIterator for &'a KonataAnnotationDetail {
    type Item = &'a KonataAnnotation;
    type IntoIter = std::slice::Iter<'a, KonataAnnotation>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone)]
struct EncodedDetailPage {
    bytes: Arc<[u8]>,
}

#[derive(Debug)]
struct CacheEntry {
    page: Arc<KonataDetailPage>,
    bytes: usize,
    last_use: u64,
}

#[derive(Debug)]
struct DetailCache {
    budget_bytes: usize,
    resident_bytes: usize,
    clock: u64,
    entries: HashMap<u32, CacheEntry>,
    pending: HashSet<u32>,
    decoding: HashSet<u32>,
    errors: HashMap<u32, Arc<str>>,
    hits: u64,
    misses: u64,
    evictions: u64,
    decode_errors: u64,
}

impl DetailCache {
    fn new(budget_bytes: usize) -> Self {
        Self {
            budget_bytes,
            resident_bytes: 0,
            clock: 0,
            entries: HashMap::new(),
            pending: HashSet::new(),
            decoding: HashSet::new(),
            errors: HashMap::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
            decode_errors: 0,
        }
    }

    fn query(&mut self, page_id: u32) -> KonataDetailQuery {
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&page_id) {
            entry.last_use = self.clock;
            self.hits = self.hits.saturating_add(1);
            return KonataDetailQuery::Ready(entry.page.clone());
        }
        if let Some(error) = self.errors.get(&page_id) {
            return KonataDetailQuery::Unavailable(error.clone());
        }
        if self.pending.contains(&page_id) {
            return KonataDetailQuery::Pending;
        }
        self.misses = self.misses.saturating_add(1);
        self.pending.insert(page_id);
        KonataDetailQuery::Pending
    }

    fn insert(&mut self, page_id: u32, page: Arc<KonataDetailPage>) {
        self.pending.remove(&page_id);
        self.decoding.remove(&page_id);
        self.errors.remove(&page_id);
        self.clock = self.clock.wrapping_add(1);
        let bytes = page.decoded_bytes();
        if let Some(previous) = self.entries.remove(&page_id) {
            self.resident_bytes = self.resident_bytes.saturating_sub(previous.bytes);
        }
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.entries.insert(
            page_id,
            CacheEntry {
                page,
                bytes,
                last_use: self.clock,
            },
        );
        self.evict_to_budget(Some(page_id));
    }

    fn fail(&mut self, page_id: u32, error: Arc<str>) {
        self.pending.remove(&page_id);
        self.decoding.remove(&page_id);
        self.errors.insert(page_id, error);
        self.decode_errors = self.decode_errors.saturating_add(1);
    }

    fn evict_to_budget(&mut self, protected: Option<u32>) {
        while self.resident_bytes > self.budget_bytes && self.entries.len() > 1 {
            let Some((&victim, _)) = self
                .entries
                .iter()
                .filter(|(page_id, _)| Some(**page_id) != protected)
                .min_by_key(|(_, entry)| entry.last_use)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.resident_bytes = self.resident_bytes.saturating_sub(entry.bytes);
                self.evictions = self.evictions.saturating_add(1);
            }
        }
    }

    fn telemetry(&self) -> KonataDetailCacheTelemetry {
        KonataDetailCacheTelemetry {
            budget_bytes: self.budget_bytes,
            encoded_bytes: 0,
            page_count: 0,
            resident_bytes: self.resident_bytes,
            resident_pages: self.entries.len(),
            pending_pages: self.pending.len(),
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            decode_errors: self.decode_errors,
        }
    }
}

#[derive(Debug)]
pub(crate) struct KonataDetailStore {
    pages: Vec<EncodedDetailPage>,
    /// Global stage ordinal at the beginning of each page, plus one terminal
    /// offset. This keeps lazy table row lookup logarithmic without decoding
    /// unrelated pages.
    page_stage_offsets: Vec<usize>,
    row_count: usize,
    stage_count: usize,
    cache: Mutex<DetailCache>,
}

impl KonataDetailStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        row_count: usize,
        stages: &[KonataStage],
        stage_offsets: &[u32],
        stage_annotations: &[KonataAnnotation],
        row_annotations: &[KonataAnnotation],
        row_annotation_offsets: &[u32],
    ) -> Result<Arc<Self>, String> {
        let page_count = row_count.div_ceil(KONATA_DETAIL_PAGE_ROWS);
        let page_stage_offsets = (0..=page_count)
            .map(|page| {
                let row = (page * KONATA_DETAIL_PAGE_ROWS).min(row_count);
                stage_offsets[row] as usize
            })
            .collect();
        let pages = (0..page_count)
            .map(|page_id| {
                let row_start = page_id * KONATA_DETAIL_PAGE_ROWS;
                let row_end = (row_start + KONATA_DETAIL_PAGE_ROWS).min(row_count);
                let mut page_stages = Vec::new();
                let mut page_stage_annotations = Vec::new();
                let mut page_stage_offsets = Vec::with_capacity(row_end - row_start + 1);
                page_stage_offsets.push(0);
                for row in row_start..row_end {
                    for stage in
                        &stages[stage_offsets[row] as usize..stage_offsets[row + 1] as usize]
                    {
                        let mut stage = stage.clone();
                        let annotations = &stage_annotations
                            [stage.annotation_start as usize..stage.annotation_end as usize];
                        stage.detail_page = page_id as u32;
                        stage.annotation_start = page_stage_annotations.len() as u32;
                        page_stage_annotations.extend_from_slice(annotations);
                        stage.annotation_end = page_stage_annotations.len() as u32;
                        page_stages.push(stage);
                    }
                    page_stage_offsets.push(page_stages.len() as u32);
                }

                let annotation_start = row_annotation_offsets[row_start] as usize;
                let annotation_end = row_annotation_offsets[row_end] as usize;
                let page_row_annotations =
                    row_annotations[annotation_start..annotation_end].to_vec();
                let page_row_annotation_offsets = row_annotation_offsets[row_start..=row_end]
                    .iter()
                    .map(|offset| offset - row_annotation_offsets[row_start])
                    .collect();
                let page = KonataDetailPage {
                    id: page_id as u32,
                    row_start,
                    row_end,
                    row_stage_offsets: page_stage_offsets,
                    stages: page_stages,
                    row_annotation_offsets: page_row_annotation_offsets,
                    row_annotations: page_row_annotations,
                    stage_annotations: page_stage_annotations,
                };
                let encoded = bincode::serialize(&page).map_err(|error| error.to_string())?;
                Ok(EncodedDetailPage {
                    bytes: lz4_flex::compress_prepend_size(&encoded).into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(Arc::new(Self {
            pages,
            page_stage_offsets,
            row_count,
            stage_count: stages.len(),
            cache: Mutex::new(DetailCache::new(DEFAULT_DETAIL_CACHE_BYTES)),
        }))
    }

    pub(crate) fn stage_count(&self) -> usize {
        self.stage_count
    }

    pub(crate) fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub(crate) fn page_for_row(&self, row: usize) -> Option<u32> {
        (row < self.row_count).then_some((row / KONATA_DETAIL_PAGE_ROWS) as u32)
    }

    pub(crate) fn stage_by_ordinal(
        self: &Arc<Self>,
        ordinal: usize,
    ) -> Option<(usize, KonataStage)> {
        if ordinal >= self.stage_count {
            return None;
        }
        let page_id = self
            .page_stage_offsets
            .partition_point(|offset| *offset <= ordinal)
            .saturating_sub(1);
        let local = ordinal.checked_sub(self.page_stage_offsets[page_id])?;
        let KonataDetailQuery::Ready(page) = self.get_blocking(page_id as u32) else {
            return None;
        };
        let stage = page.stages.get(local)?.clone();
        let row_in_page = page
            .row_stage_offsets
            .partition_point(|offset| *offset as usize <= local)
            .saturating_sub(1);
        Some((page.row_start + row_in_page, stage))
    }

    pub(crate) fn query(self: &Arc<Self>, page_id: u32) -> KonataDetailQuery {
        #[cfg(not(target_arch = "wasm32"))]
        if tokio::runtime::Handle::try_current().is_err() {
            // Headless callers without an executor cannot schedule a miss.
            // They are never a frame-painting context, so decoding here keeps
            // pure model/unit-test use deterministic.
            return self.get_blocking(page_id);
        }
        let query = self.cache.lock().map_or_else(
            |_| KonataDetailQuery::Unavailable(Arc::from("Konata detail cache lock poisoned")),
            |mut cache| cache.query(page_id),
        );
        if matches!(query, KonataDetailQuery::Pending) {
            let should_schedule = self.cache.lock().is_ok_and(|mut cache| {
                cache.pending.contains(&page_id)
                    && !cache.entries.contains_key(&page_id)
                    && !cache.errors.contains_key(&page_id)
                    && cache.decoding.insert(page_id)
            });
            if should_schedule {
                self.schedule_decode(page_id);
            }
        }
        query
    }

    pub(crate) fn get_blocking(self: &Arc<Self>, page_id: u32) -> KonataDetailQuery {
        if let Ok(mut cache) = self.cache.lock() {
            match cache.query(page_id) {
                ready @ KonataDetailQuery::Ready(_) | ready @ KonataDetailQuery::Unavailable(_) => {
                    return ready;
                }
                KonataDetailQuery::Pending => {}
            }
        }
        match self.decode(page_id) {
            Ok(page) => {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.insert(page_id, page.clone());
                }
                KonataDetailQuery::Ready(page)
            }
            Err(error) => {
                let error: Arc<str> = error.into();
                if let Ok(mut cache) = self.cache.lock() {
                    cache.fail(page_id, error.clone());
                }
                KonataDetailQuery::Unavailable(error)
            }
        }
    }

    fn schedule_decode(self: &Arc<Self>, page_id: u32) {
        // Only the first miss owns the pending marker. Re-checking query must
        // not launch duplicate work.
        let decode = self.clone();
        OUTSTANDING_TRANSACTIONS.fetch_add(1, Ordering::SeqCst);
        crate::async_util::perform_work(move || {
            match decode.decode(page_id) {
                Ok(page) => {
                    if let Ok(mut cache) = decode.cache.lock() {
                        cache.insert(page_id, page);
                    }
                }
                Err(error) => {
                    if let Ok(mut cache) = decode.cache.lock() {
                        cache.fail(page_id, error.into());
                    }
                }
            }
            OUTSTANDING_TRANSACTIONS.fetch_sub(1, Ordering::SeqCst);
            if let Ok(context) = EGUI_CONTEXT.read()
                && let Some(context) = context.as_ref()
            {
                context.request_repaint();
            }
        });
    }

    fn decode(&self, page_id: u32) -> Result<Arc<KonataDetailPage>, String> {
        let encoded = self
            .pages
            .get(page_id as usize)
            .ok_or_else(|| format!("Konata detail page {page_id} is unavailable"))?;
        let bytes = lz4_flex::decompress_size_prepended(&encoded.bytes).map_err(|error| {
            format!("Failed to decompress Konata detail page {page_id}: {error}")
        })?;
        let page: KonataDetailPage = bincode::deserialize(&bytes)
            .map_err(|error| format!("Failed to decode Konata detail page {page_id}: {error}"))?;
        if page.id != page_id {
            return Err(format!(
                "Konata detail page identity mismatch: requested {page_id}, decoded {}",
                page.id
            ));
        }
        Ok(Arc::new(page))
    }

    pub(crate) fn row_detail(self: &Arc<Self>, row: usize, blocking: bool) -> KonataRowDetail {
        let Some(page_id) = self.page_for_row(row) else {
            return KonataRowDetail::empty();
        };
        let query = if blocking {
            self.get_blocking(page_id)
        } else {
            self.query(page_id)
        };
        match query {
            KonataDetailQuery::Ready(page) => {
                KonataRowDetail::new(page, row).unwrap_or_else(KonataRowDetail::empty)
            }
            KonataDetailQuery::Pending | KonataDetailQuery::Unavailable(_) => {
                KonataRowDetail::empty()
            }
        }
    }

    pub(crate) fn row_annotations(
        self: &Arc<Self>,
        row: usize,
        blocking: bool,
    ) -> Option<KonataAnnotationDetail> {
        let page_id = self.page_for_row(row)?;
        let query = if blocking {
            self.get_blocking(page_id)
        } else {
            self.query(page_id)
        };
        match query {
            KonataDetailQuery::Ready(page) => {
                let annotations = page.row_annotations_range(row)?;
                Some(KonataAnnotationDetail { page, annotations })
            }
            KonataDetailQuery::Pending | KonataDetailQuery::Unavailable(_) => None,
        }
    }

    pub(crate) fn prefetch_rows(self: &Arc<Self>, rows: Range<usize>) {
        if rows.start >= rows.end || self.row_count == 0 {
            return;
        }
        let first = rows.start.min(self.row_count - 1) / KONATA_DETAIL_PAGE_ROWS;
        let last = rows.end.saturating_sub(1).min(self.row_count - 1) / KONATA_DETAIL_PAGE_ROWS;
        for page_id in first..=last {
            let _ = self.query(page_id as u32);
        }
    }

    pub(crate) fn telemetry(&self) -> KonataDetailCacheTelemetry {
        let mut telemetry = self.cache.lock().map_or_else(
            |_| KonataDetailCacheTelemetry::default(),
            |cache| cache.telemetry(),
        );
        telemetry.encoded_bytes = self.pages.iter().map(|page| page.bytes.len()).sum();
        telemetry.page_count = self.pages.len();
        telemetry
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.cache
            .lock()
            .is_ok_and(|cache| cache.pending.is_empty())
    }

    pub(crate) fn set_budget(&self, budget_bytes: usize) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.budget_bytes = budget_bytes.max(1);
            cache.evict_to_budget(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konata::{KonataScalar, StageFlags};

    fn three_page_store() -> Arc<KonataDetailStore> {
        let row_count = KONATA_DETAIL_PAGE_ROWS * 2 + 1;
        let stage_rows = [0, KONATA_DETAIL_PAGE_ROWS, row_count - 1];
        let mut stages = Vec::new();
        let mut offsets = Vec::with_capacity(row_count + 1);
        offsets.push(0);
        for row in 0..row_count {
            if stage_rows.contains(&row) {
                stages.push(KonataStage {
                    start: row as u64,
                    end: row as u64 + 1,
                    event_tx: row as u64 + 10,
                    name: 0,
                    lane: 0,
                    flags: StageFlags::default(),
                    detail_page: 0,
                    annotation_start: 0,
                    annotation_end: 1,
                });
            }
            offsets.push(stages.len() as u32);
        }
        let stage_annotations = vec![KonataAnnotation {
            name: 0,
            value: KonataScalar::Unsigned(1),
        }];
        let row_annotation_offsets = vec![0; row_count + 1];
        KonataDetailStore::encode(
            row_count,
            &stages,
            &offsets,
            &stage_annotations,
            &[],
            &row_annotation_offsets,
        )
        .unwrap()
    }

    #[test]
    fn forced_eviction_keeps_decoded_detail_under_budget() {
        let store = three_page_store();
        let first = store.get_blocking(0);
        let KonataDetailQuery::Ready(first) = first else {
            panic!("first detail page should decode");
        };
        store.set_budget(first.decoded_bytes());
        assert!(matches!(store.get_blocking(1), KonataDetailQuery::Ready(_)));
        assert!(matches!(store.get_blocking(2), KonataDetailQuery::Ready(_)));
        let telemetry = store.telemetry();
        assert_eq!(telemetry.resident_pages, 1);
        assert!(telemetry.resident_bytes <= telemetry.budget_bytes);
        assert!(telemetry.evictions >= 2);
        assert!(matches!(
            store.get_blocking(99),
            KonataDetailQuery::Unavailable(_)
        ));
    }

    #[tokio::test]
    async fn nonblocking_miss_reports_pending_then_publishes_page() {
        let store = three_page_store();
        assert!(matches!(store.query(0), KonataDetailQuery::Pending));
        for _ in 0..100 {
            if matches!(store.query(0), KonataDetailQuery::Ready(_)) {
                assert!(store.is_idle());
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("background detail decode did not publish its page");
    }

    #[test]
    fn paged_csr_round_trips_detail_across_page_boundaries() {
        let row_count = KONATA_DETAIL_PAGE_ROWS * 2 + 17;
        let mut stages = Vec::new();
        let mut stage_offsets = Vec::with_capacity(row_count + 1);
        let mut stage_annotations = Vec::new();
        stage_offsets.push(0);
        for row in 0..row_count {
            for ordinal in 0..=row % 3 {
                let annotation_start = stage_annotations.len() as u32;
                stage_annotations.push(KonataAnnotation {
                    name: ordinal as u32,
                    value: KonataScalar::Unsigned((row * 10 + ordinal) as u64),
                });
                stages.push(KonataStage {
                    start: (row * 10 + ordinal) as u64,
                    end: (row * 10 + ordinal + 1) as u64,
                    event_tx: (row * 8 + ordinal) as u64,
                    name: ordinal as u16,
                    lane: (row % 2) as u16,
                    flags: StageFlags::default(),
                    detail_page: u32::MAX,
                    annotation_start,
                    annotation_end: annotation_start + 1,
                });
            }
            stage_offsets.push(stages.len() as u32);
        }
        let row_annotations = (0..row_count)
            .map(|row| KonataAnnotation {
                name: 99,
                value: KonataScalar::Unsigned(row as u64),
            })
            .collect::<Vec<_>>();
        let row_annotation_offsets = (0..=row_count).map(|row| row as u32).collect::<Vec<_>>();
        let store = KonataDetailStore::encode(
            row_count,
            &stages,
            &stage_offsets,
            &stage_annotations,
            &row_annotations,
            &row_annotation_offsets,
        )
        .unwrap();

        for row in [
            0,
            1,
            KONATA_DETAIL_PAGE_ROWS - 1,
            KONATA_DETAIL_PAGE_ROWS,
            KONATA_DETAIL_PAGE_ROWS + 1,
            row_count - 1,
        ] {
            let expected = &stages[stage_offsets[row] as usize..stage_offsets[row + 1] as usize];
            let detail = store.row_detail(row, true);
            assert_eq!(detail.len(), expected.len());
            for (ordinal, (actual, expected)) in detail.iter().zip(expected).enumerate() {
                assert_eq!(
                    (
                        actual.start,
                        actual.end,
                        actual.event_tx,
                        actual.name,
                        actual.lane
                    ),
                    (
                        expected.start,
                        expected.end,
                        expected.event_tx,
                        expected.name,
                        expected.lane
                    )
                );
                assert_eq!(actual.detail_page as usize, row / KONATA_DETAIL_PAGE_ROWS);
                assert_eq!(
                    detail.annotations_for_stage(actual),
                    vec![KonataAnnotation {
                        name: expected.name as u32,
                        value: KonataScalar::Unsigned(expected.start),
                    }]
                );
                let (resolved_row, resolved) = store
                    .stage_by_ordinal(stage_offsets[row] as usize + ordinal)
                    .unwrap();
                assert_eq!(resolved_row, row);
                assert_eq!(resolved.event_tx, actual.event_tx);
            }
            let annotations = store.row_annotations(row, true).unwrap();
            assert_eq!(
                &*annotations,
                &[KonataAnnotation {
                    name: 99,
                    value: KonataScalar::Unsigned(row as u64),
                }]
            );
        }
    }

    #[test]
    #[ignore = "multi-million-row scale acceptance"]
    fn multi_million_rows_keep_decoded_detail_bounded_under_forced_eviction() {
        const ROWS: usize = 2_000_000;
        let started = std::time::Instant::now();
        let stages = (0..ROWS)
            .map(|row| KonataStage {
                start: row as u64 * 2,
                end: row as u64 * 2 + 1,
                event_tx: row as u64 + 1,
                name: (row % 12) as u16,
                lane: (row % 2) as u16,
                flags: StageFlags::default(),
                detail_page: 0,
                annotation_start: 0,
                annotation_end: 0,
            })
            .collect::<Vec<_>>();
        let offsets = (0..=ROWS).map(|row| row as u32).collect::<Vec<_>>();
        let row_annotation_offsets = vec![0; ROWS + 1];
        let store =
            KonataDetailStore::encode(ROWS, &stages, &offsets, &[], &[], &row_annotation_offsets)
                .unwrap();
        let build_time = started.elapsed();
        assert_eq!(store.page_count(), ROWS.div_ceil(KONATA_DETAIL_PAGE_ROWS));
        let first = match store.get_blocking(0) {
            KonataDetailQuery::Ready(page) => page,
            _ => panic!("first scale page should decode"),
        };
        store.set_budget(first.decoded_bytes());
        for page in [store.page_count() / 2, store.page_count() - 1, 0] {
            assert!(matches!(
                store.get_blocking(page as u32),
                KonataDetailQuery::Ready(_)
            ));
        }
        let telemetry = store.telemetry();
        assert_eq!(telemetry.resident_pages, 1);
        assert!(telemetry.resident_bytes <= telemetry.budget_bytes);
        assert!(telemetry.evictions >= 3);
        assert!(telemetry.encoded_bytes < ROWS * 24 + (ROWS + 1) * 8);
        eprintln!(
            "Konata scale acceptance: {ROWS} rows/stages, {} pages, {} encoded MiB, build {build_time:?}, forced cache {} KiB",
            telemetry.page_count,
            telemetry.encoded_bytes / (1024 * 1024),
            telemetry.budget_bytes / 1024,
        );
    }
}
