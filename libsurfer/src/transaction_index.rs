//! Immutable complete-track geometry, independent of the current viewport.
//!
//! Build once in a loader. Queries visit visible rows and occupied screen bins,
//! never all transactions in the time window. Rich payloads remain elsewhere.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TrackKey {
    pub stream: u32,
    pub generator: Option<u32>,
}

impl From<&crate::transaction_container::TransactionStreamRef> for TrackKey {
    fn from(reference: &crate::transaction_container::TransactionStreamRef) -> Self {
        Self {
            stream: reference.stream_id.0 as u32,
            generator: reference.gen_id.map(|g| g.0 as u32),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub id: u64,
    pub begin: u64,
    pub end: u64,
    pub generator: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sample {
    pub row: usize,
    pub begin: u64,
    pub end: u64,
    /// The final member preserves long intervals starting in a dense pixel.
    pub representative: Span,
    pub count: usize,
}

pub(crate) struct TrackIndex {
    spans: Vec<Span>,
    rows: Vec<Vec<usize>>,
}

impl TrackIndex {
    pub fn new(mut spans: Vec<Span>) -> Self {
        spans.sort_unstable_by_key(|span| (span.begin, span.id));
        let mut rows: Vec<Vec<usize>> = Vec::new();
        let mut busy: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
        let mut free: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
        for (index, span) in spans.iter().enumerate() {
            while let Some(&Reverse((end, row))) = busy.peek() {
                if end > span.begin {
                    break;
                }
                busy.pop();
                free.push(Reverse(row));
            }
            let row = free.pop().map_or_else(
                || {
                    rows.push(Vec::new());
                    rows.len() - 1
                },
                |Reverse(row)| row,
            );
            rows[row].push(index);
            busy.push(Reverse((span.end.max(span.begin), row)));
        }
        Self { spans, rows }
    }

    /// Chronological navigation does not materialize all IDs or require payloads.
    pub fn neighbor(&self, anchor: Option<(u64, u64)>, next: bool) -> Option<Span> {
        let Some(anchor) = anchor else {
            return self.spans.first().copied();
        };
        let index = if next {
            self.spans
                .partition_point(|span| (span.begin, span.id) <= anchor)
        } else {
            self.spans
                .partition_point(|span| (span.begin, span.id) < anchor)
                .checked_sub(1)?
        };
        self.spans.get(index).copied()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn query(&self, rows: Range<usize>, begin: u64, end: u64, pixels: usize) -> Vec<Sample> {
        let mut samples = Vec::new();
        for row in rows.start.min(self.rows.len())..rows.end.min(self.rows.len()) {
            let indices = &self.rows[row];
            sample_row(
                indices.len(),
                |index| self.spans[indices[index]],
                row,
                begin,
                end,
                pixels,
                &mut samples,
            );
        }
        samples
    }
}

// Use integer arithmetic for exact bins even beyond f64 timestamp precision.
// The accessor also lets tests exercise a billion-element row without allocating
// a billion payloads; production accesses the same algorithm through its index.
fn sample_row(
    len: usize,
    mut at: impl FnMut(usize) -> Span,
    row: usize,
    begin: u64,
    end: u64,
    pixels: usize,
    out: &mut Vec<Sample>,
) {
    if pixels == 0 || begin > end {
        return;
    }
    let partition = |lo: usize, predicate: &mut dyn FnMut(usize) -> bool| {
        let (mut low, mut high) = (lo, len);
        while low < high {
            let mid = low + (high - low) / 2;
            if predicate(mid) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    };
    let mut index = partition(0, &mut |i| at(i).end < begin);
    let duration = u128::from(end) - u128::from(begin) + 1;
    while index < len {
        let first = at(index);
        if first.begin > end {
            break;
        }
        let bin = (u128::from(first.begin.saturating_sub(begin)) * pixels as u128 / duration)
            .min(pixels as u128 - 1);
        // Starts before the next bin boundary round upward in integer time.
        let next_time = u128::from(begin) + ((bin + 1) * duration).div_ceil(pixels as u128);
        let next = partition(index + 1, &mut |i| u128::from(at(i).begin) < next_time);
        let last = at(next - 1);
        out.push(Sample {
            row,
            begin: first.begin,
            end: last.end,
            representative: last,
            count: next - index,
        });
        index = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: u64, begin: u64, end: u64) -> Span {
        Span {
            id,
            begin,
            end,
            generator: 1,
        }
    }

    #[test]
    fn stable_rows_do_not_depend_on_input_order_or_window() {
        let spans = vec![span(9, 1, 100), span(2, 2, 4), span(1, 2, 3), span(3, 5, 8)];
        let index = TrackIndex::new(spans.clone());
        let reversed = TrackIndex::new(spans.into_iter().rev().collect());
        assert_eq!(index.row_count(), 3);
        let full = index.query(0..3, 0, 100, 101);
        assert_eq!(full, reversed.query(0..3, 0, 100, 101));
        for (begin, end) in [(2, 3), (5, 8), (90, 95)] {
            let window = index.query(0..3, begin, end, 101);
            for item in window {
                assert_eq!(
                    item.row,
                    full.iter()
                        .find(|x| x.representative.id == item.representative.id)
                        .unwrap()
                        .row
                );
            }
        }
        assert_eq!(index.query(0..1, 90, 95, 100)[0].representative.id, 9);
    }

    #[test]
    fn navigation_uses_time_and_identity_without_payload_materialization() {
        let index = TrackIndex::new(vec![
            span(1, 100, 110),
            span(9, 10, 200),
            span(2, 20, 21),
            span(3, 20, 22),
        ]);
        assert_eq!(index.neighbor(None, true).unwrap().id, 9);
        assert_eq!(index.neighbor(Some((10, 9)), true).unwrap().id, 2);
        assert_eq!(index.neighbor(Some((20, 2)), true).unwrap().id, 3);
        assert_eq!(index.neighbor(Some((20, 3)), false).unwrap().id, 2);
        assert!(index.neighbor(Some((100, 1)), true).is_none());
        assert!(index.neighbor(Some((10, 9)), false).is_none());
    }

    #[test]
    fn dense_pixel_keeps_long_last_member_and_counts_every_member() {
        let index = TrackIndex::new(vec![span(1, 1, 2), span(2, 2, 3), span(3, 3, 900)]);
        let samples = index.query(0..1, 0, 1000, 10);
        assert_eq!(
            samples,
            [Sample {
                row: 0,
                begin: 1,
                end: 900,
                representative: span(3, 3, 900),
                count: 3
            }]
        );
        assert_eq!(index.query(0..1, 500, 600, 100)[0].representative.id, 3);
    }

    #[test]
    fn billion_transactions_query_is_bounded_by_pixels() {
        let mut samples = Vec::new();
        let mut accesses = 0;
        sample_row(
            1_000_000_000,
            |i| {
                accesses += 1;
                span(i as u64, i as u64 * 2, i as u64 * 2 + 1)
            },
            0,
            0,
            2_000_000_000,
            1920,
            &mut samples,
        );
        assert!(samples.len() <= 1920);
        assert_eq!(
            samples.iter().map(|s| s.count).sum::<usize>(),
            1_000_000_000
        );
        assert!(accesses < 1920 * 34, "{accesses} metadata accesses");
    }

    #[test]
    fn exact_large_timestamps_endpoints_and_empty_queries() {
        let index = TrackIndex::new(vec![span(1, u64::MAX - 2, u64::MAX)]);
        assert_eq!(
            index.query(0..1, u64::MAX, u64::MAX, 1)[0]
                .representative
                .id,
            1
        );
        assert!(index.query(1..100, 0, u64::MAX, 100).is_empty());
        assert!(index.query(0..1, 0, u64::MAX, 0).is_empty());
        assert!(index.query(0..1, 2, 1, 100).is_empty());
        assert!(TrackIndex::new(vec![]).query(0..1, 0, 10, 10).is_empty());
    }
    #[test]
    #[ignore = "manual timing comparison; cargo test transaction_index_render_benchmark -- --ignored --nocapture"]
    fn transaction_index_render_benchmark() {
        use ftr_parser::types::{Event, GeneratorId, Transaction, TransactionId};
        use std::hint::black_box;
        use std::time::Instant;
        let count = 1_000_000u64;
        let transactions: Vec<_> = (0..count)
            .map(|i| Transaction {
                event: Event {
                    tx_id: TransactionId(i as usize),
                    gen_id: GeneratorId(1),
                    start_time: (2 * i).into(),
                    end_time: (2 * i + 1).into(),
                },
                attributes: vec![],
                inc_relations: vec![],
                out_relations: vec![],
                row: 0,
            })
            .collect();
        let start = Instant::now();
        let index = TrackIndex::new((0..count).map(|i| span(i, 2 * i, 2 * i + 1)).collect());
        let build = start.elapsed();
        let mut old_best = std::time::Duration::MAX;
        let mut indexed_best = std::time::Duration::MAX;
        let mut visible = 0;
        for _ in 0..3 {
            let start = Instant::now();
            black_box(crate::transactions::packet_rows(black_box(
                transactions.iter(),
            )));
            old_best = old_best.min(start.elapsed());
            let start = Instant::now();
            let result = black_box(index.query(0..1, 0, count * 2, 1920));
            visible = result.len();
            indexed_best = indexed_best.min(start.elapsed());
        }
        println!(
            "records={count}, index_build_ms={:.3}, previous_row_repack_ms={:.3}, indexed_query_ms={:.3}, screen_samples={visible}",
            build.as_secs_f64() * 1000.0,
            old_best.as_secs_f64() * 1000.0,
            indexed_best.as_secs_f64() * 1000.0
        );
        assert!(visible <= 1920);
    }
}
