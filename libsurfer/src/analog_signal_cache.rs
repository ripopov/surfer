//! Analog signal cache with fast min/max queries using Blocked RMQ + Binary Search
//!
//! # Complexity
//!
//! - Construction: O(N + N/B · log(N/B))
//! - Time queries: O(log N) for binary search + O(1) for min/max
//!
//! - Memory: O(N + N/B · log(N/B)) where B is the block size
//!
//! Per-sample storage (N samples):
//! - `timestamps: Vec<u64>` = 8N bytes
//! - `values: Vec<f64>` = 8N bytes
//!
//! Sparse table storage (B = 64 default block size):
//! - `MinMax` struct = 24 bytes (8 + 8 + 1 + padding)
//! - `num_blocks` = ⌈N/B⌉
//! - `num_levels` = 1 + ⌊`log₂(num_blocks)`⌋
//! - Sparse table = `num_blocks` × `num_levels` × 24 bytes
//!
//! Total ≈ 16N + (N/64) × (1 + log₂(N/64)) × 24 bytes
//!
//! # Examples
//!   1M Samples ~ 21.3 MB
//!   100M Samples ~ 2.39 GB
//!

use crate::translation::DynTranslator;
use crate::wave_container::{AnalogCacheKey, SignalAccessor, VariableMeta};
use num::ToPrimitive;
use std::borrow::Cow;
use std::sync::OnceLock;
use surfer_translation_types::{ValueKind, ValueRepr, VariableValue};

/// Quiet NaN representing undefined (X) values.
pub const NAN_UNDEF: f64 = f64::from_bits(0x7FF8_0000_0000_0000_u64);

/// Quiet NaN representing high-impedance (Z) values.
pub const NAN_HIGHIMP: f64 = f64::from_bits(0x7FF8_0000_0000_0001_u64);

/// Check NaN payload to determine if it represents `HighImp`.
#[must_use]
pub fn is_nan_highimp(value: f64) -> bool {
    value.to_bits() == NAN_HIGHIMP.to_bits()
}

#[derive(Clone, Copy, PartialEq)]
struct MinMax {
    min: f64,
    max: f64,
    has_non_finite: bool,
}

impl MinMax {
    fn new(value: f64) -> Self {
        if value.is_finite() {
            Self {
                min: value,
                max: value,
                has_non_finite: false,
            }
        } else {
            // Use identity values for min/max operations:
            // INFINITY.min(x) == x for any finite x
            // NEG_INFINITY.max(x) == x for any finite x
            Self {
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                has_non_finite: true,
            }
        }
    }

    fn combine(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            has_non_finite: self.has_non_finite || other.has_non_finite,
        }
    }

    fn from_slice(values: &[f64]) -> Self {
        values
            .iter()
            .fold(Self::new(values[0]), |acc, &v| acc.combine(&Self::new(v)))
    }
}

pub struct CacheQueryResult {
    pub current: Option<(u64, f64)>,
    pub next: Option<u64>,
}

struct SignalRMQ {
    timestamps: Vec<u64>,
    values: Vec<f64>,
    block_size: usize,
    /// `sparse_table`[level][block_idx] contains min/max for 2^level blocks starting at `block_idx`
    /// Level 0 contains individual block summaries.
    sparse_table: Vec<Vec<MinMax>>,
}

impl SignalRMQ {
    fn new(signal: impl IntoIterator<Item = (u64, f64)>, block_size: usize) -> Self {
        let data = signal.into_iter().collect::<Vec<_>>();

        debug_assert!(
            data.windows(2).all(|w| w[1].0 > w[0].0),
            "Timestamps must be strictly increasing"
        );

        let (timestamps, values) = data.into_iter().unzip::<u64, f64, Vec<_>, Vec<_>>();

        let num_blocks = values.len().div_ceil(block_size);

        let block_summaries = (0..num_blocks)
            .map(|block_idx| {
                let start = block_idx * block_size;
                let end = (start + block_size).min(values.len());
                MinMax::from_slice(&values[start..end])
            })
            .collect::<Vec<_>>();

        let sparse_table = Self::build_sparse_table(&block_summaries);

        Self {
            timestamps,
            values,
            block_size,
            sparse_table,
        }
    }

    /// Builds a sparse table for O(1) range min/max queries over blocks.
    ///
    /// `table[k][i]` stores min/max over up to 2^k consecutive blocks starting at block i.
    /// Any range query can be answered by combining at most two overlapping entries.
    fn build_sparse_table(block_summaries: &[MinMax]) -> Vec<Vec<MinMax>> {
        let num_blocks = block_summaries.len();
        let num_levels = num_blocks.ilog2() as usize;

        // Level 0 contains individual block summaries
        let mut table = vec![block_summaries.to_vec()];

        for level in 0..num_levels {
            let prev = &table[level];
            let span = 1 << level; // Each entry covers 2^level blocks

            // Level (level+1) combines pairs of entries from current level
            let next_level = (0..num_blocks)
                .map(|i| {
                    let left = prev[i];
                    // If (i + span) is out of bounds, just use 'left' (self-combine/no-op).
                    prev.get(i + span).map_or(left, |right| left.combine(right))
                })
                .collect::<Vec<_>>();

            table.push(next_level);
        }

        table
    }

    fn query_time_range(&self, t_start: u64, t_end: u64) -> Option<MinMax> {
        if t_start > t_end {
            return None;
        }

        let l = self
            .timestamps
            .binary_search(&t_start)
            .unwrap_or_else(|x| x);
        let r = match self.timestamps.binary_search(&t_end) {
            Ok(idx) => idx,
            Err(idx) => {
                if idx == 0 {
                    return None;
                }
                idx - 1
            }
        };

        if l > r || l >= self.values.len() {
            return None;
        }

        Some(self.query_index_range(l, r))
    }

    fn query_index_range(&self, l: usize, r: usize) -> MinMax {
        debug_assert!(l <= r && r < self.values.len(), "Invalid index range");

        let l_block = l / self.block_size;
        let r_block = r / self.block_size;

        if l_block == r_block {
            return MinMax::from_slice(&self.values[l..=r]);
        }

        // Left partial block: from l to end of l_block (or r if smaller)
        let l_block_end = (l_block + 1) * self.block_size - 1;
        let mut result = MinMax::from_slice(&self.values[l..=l_block_end.min(r)]);

        // Right partial block: from start of r_block to r
        let r_block_start = r_block * self.block_size;
        if r_block > l_block {
            let partial = MinMax::from_slice(&self.values[r_block_start..=r]);
            result = result.combine(&partial);
        }

        // Full blocks in the middle
        let first_full_block = l_block + 1;
        let last_full_block = r_block - 1;

        if first_full_block <= last_full_block {
            let middle = self.query_blocks(first_full_block, last_full_block);
            result = result.combine(&middle);
        }

        result
    }

    fn query_blocks(&self, l_block: usize, r_block: usize) -> MinMax {
        debug_assert!(
            l_block <= r_block,
            "query_blocks called with l_block > r_block"
        );

        if l_block == r_block {
            return self.sparse_table[0][l_block];
        }

        let range_len = r_block - l_block + 1;
        let level = range_len.ilog2() as usize;
        let jump = 1 << level;

        let left = self.sparse_table[level][l_block];
        let right = self.sparse_table[level][r_block - jump + 1];

        left.combine(&right)
    }

    fn time_range(&self) -> Option<(u64, u64)> {
        if self.timestamps.is_empty() {
            None
        } else {
            Some((self.timestamps[0], *self.timestamps.last().unwrap()))
        }
    }

    /// Query the signal value at a specific time.
    ///
    /// Returns the value at or before the query time, along with the next transition time.
    /// If the query time is before the first sample, `current` is `None` but `next` points
    /// to the first sample.
    fn query_at_time(&self, time: u64) -> CacheQueryResult {
        match self.timestamps.binary_search(&time) {
            Ok(idx) => CacheQueryResult {
                current: Some((self.timestamps[idx], self.values[idx])),
                next: self.timestamps.get(idx + 1).copied(),
            },
            Err(0) => CacheQueryResult {
                current: None, // Before first sample
                next: self.timestamps.first().copied(),
            },
            Err(idx) => CacheQueryResult {
                current: Some((self.timestamps[idx - 1], self.values[idx - 1])),
                next: self.timestamps.get(idx).copied(),
            },
        }
    }
}

/// Cache entry for a single analog signal.
pub struct AnalogSignalCache {
    rmq: SignalRMQ,
    pub global_min: f64,
    pub global_max: f64,
    /// Total number of time units (for cache invalidation on reload).
    pub num_timestamps: u64,
}

impl AnalogSignalCache {
    pub fn build(
        accessor: SignalAccessor,
        translator: &DynTranslator,
        meta: &VariableMeta,
        num_timestamps: u64,
        block_size: Option<usize>,
    ) -> Option<Self> {
        let block_size = block_size.unwrap_or(64);

        let mut signal_data = Vec::new();

        for (time_u64, var_value) in accessor.iter_changes() {
            let numeric = translate_to_numeric(translator, meta, &var_value).unwrap_or(f64::NAN);
            signal_data.push((time_u64, numeric));
        }

        if signal_data.is_empty() {
            return None;
        }

        let rmq = SignalRMQ::new(signal_data, block_size);
        let (first_time, last_time) = rmq.time_range()?;
        let global = rmq.query_time_range(first_time, last_time)?;

        Some(Self {
            rmq,
            global_min: global.min,
            global_max: global.max,
            num_timestamps,
        })
    }

    #[must_use]
    pub fn query_time_range(&self, start: u64, end: u64) -> Option<(f64, f64)> {
        let result = self.rmq.query_time_range(start, end)?;
        if result.has_non_finite {
            // Propagate NaN so renderer draws undefined for ranges containing non-finite values
            Some((result.min, NAN_UNDEF))
        } else {
            Some((result.min, result.max))
        }
    }

    #[must_use]
    pub fn query_at_time(&self, time: u64) -> CacheQueryResult {
        self.rmq.query_at_time(time)
    }
}

/// Wrapper for analog cache with reference counting and lazy initialization.
///
/// Used for cache sharing between variables with the same signal+translator combo.
/// The cache is built asynchronously and set via `OnceLock` when ready.
pub struct AnalogCacheEntry {
    inner: OnceLock<AnalogSignalCache>,
    pub cache_key: AnalogCacheKey,
    pub generation: u64,
}

impl AnalogCacheEntry {
    #[must_use]
    pub fn new(cache_key: AnalogCacheKey, generation: u64) -> Self {
        Self {
            inner: OnceLock::new(),
            cache_key,
            generation,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.inner.get().is_some()
    }

    pub fn get(&self) -> Option<&AnalogSignalCache> {
        self.inner.get()
    }

    pub fn set(&self, cache: AnalogSignalCache) {
        let _ = self.inner.set(cache);
    }
}

pub fn translate_to_numeric(
    translator: &DynTranslator,
    meta: &VariableMeta,
    value: &VariableValue,
) -> Option<f64> {
    let translation = translator.translate(meta, value).ok()?;

    // Check ValueKind first - if it's HighImp or Undef, return appropriate NaN
    match translation.kind {
        ValueKind::HighImp => return Some(NAN_HIGHIMP),
        ValueKind::Undef => return Some(NAN_UNDEF),
        _ => {}
    }

    // Try to parse as numeric value
    let value_str: Cow<str> = match &translation.val {
        ValueRepr::Bit(c) => Cow::Owned(c.to_string()),
        ValueRepr::Bits(_, s) => Cow::Borrowed(s),
        ValueRepr::String(s) => Cow::Borrowed(s),
        _ => return None,
    };
    parse_numeric_value(&value_str, &translator.name())
}

fn biguint_to_f64(v: &num::BigUint) -> f64 {
    // For small values that fit in u64, use direct conversion
    if let Some(val) = v.to_u64() {
        return val as f64;
    }
    v.to_string().parse::<f64>().unwrap_or(f64::INFINITY)
}

fn parse_numeric_value(s: &str, translator_name: &str) -> Option<f64> {
    let s = s.trim();
    let translator_lower = translator_name.to_lowercase();

    if translator_lower.contains("hex") {
        let hex_str = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(s);
        num::BigUint::parse_bytes(hex_str.as_bytes(), 16).map(|v| biguint_to_f64(&v))
    } else if translator_lower.contains("bin") {
        let bin_str = s
            .strip_prefix("0b")
            .or_else(|| s.strip_prefix("0B"))
            .unwrap_or(s);
        num::BigUint::parse_bytes(bin_str.as_bytes(), 2).map(|v| biguint_to_f64(&v))
    } else {
        if let Ok(v) = s.parse::<f64>() {
            return Some(v);
        }
        // Fallback: try parsing as hex for non-decimal strings
        num::BigUint::parse_bytes(s.as_bytes(), 16).map(|v| biguint_to_f64(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_sample() {
        let signal = vec![(100, 5.0)];
        let rmq = SignalRMQ::new(signal, 64);

        assert_eq!(rmq.timestamps.len(), 1);
        assert_eq!(rmq.time_range(), Some((100, 100)));

        let result = rmq.query_time_range(100, 100).unwrap();
        assert_eq!(result.min, 5.0);
        assert_eq!(result.max, 5.0);

        assert!(rmq.query_time_range(0, 99).is_none());
        assert!(rmq.query_time_range(101, 200).is_none());
    }

    #[test]
    fn test_two_samples() {
        let signal = vec![(10, 3.0), (20, 7.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(10, 20).unwrap();
        assert_eq!(result.min, 3.0);
        assert_eq!(result.max, 7.0);

        let result = rmq.query_time_range(10, 10).unwrap();
        assert_eq!(result.min, 3.0);
        assert_eq!(result.max, 3.0);

        let result = rmq.query_time_range(20, 20).unwrap();
        assert_eq!(result.min, 7.0);
        assert_eq!(result.max, 7.0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "Timestamps must be strictly increasing")]
    fn test_unsorted_input() {
        let signal = vec![(20, 7.0), (10, 3.0), (15, 5.0)];
        SignalRMQ::new(signal, 64);
    }

    #[test]
    fn test_irregular_timestamps() {
        let signal = vec![
            (1, 1.0),
            (37, 5.0),
            (41, 2.0),
            (512, 8.0),
            (513, 3.0),
            (2080, 6.0),
        ];
        let rmq = SignalRMQ::new(signal, 2);

        // Query full range
        let result = rmq.query_time_range(1, 2080).unwrap();
        assert_eq!(result.min, 1.0);
        assert_eq!(result.max, 8.0);

        // Query subrange
        let result = rmq.query_time_range(37, 513).unwrap();
        assert_eq!(result.min, 2.0);
        assert_eq!(result.max, 8.0);

        // Query exact timestamp
        let result = rmq.query_time_range(512, 512).unwrap();
        assert_eq!(result.min, 8.0);
        assert_eq!(result.max, 8.0);
    }

    #[test]
    fn test_large_signal_multiple_blocks() {
        let mut signal = Vec::new();
        for i in 0..1000 {
            signal.push((i as u64, f64::from(i % 100)));
        }

        let rmq = SignalRMQ::new(signal, 32);

        // Query full range
        let result = rmq.query_time_range(0, 999).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 99.0);

        // Query first block
        let result = rmq.query_time_range(0, 31).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 31.0);

        // Query spanning blocks
        let result = rmq.query_time_range(50, 150).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 99.0);
    }

    #[test]
    fn test_all_same_values() {
        let signal: Vec<_> = (0..100).map(|i| (i as u64, 42.0)).collect();
        let rmq = SignalRMQ::new(signal, 32);

        let result = rmq.query_time_range(0, 99).unwrap();
        assert_eq!(result.min, 42.0);
        assert_eq!(result.max, 42.0);

        let result = rmq.query_time_range(25, 75).unwrap();
        assert_eq!(result.min, 42.0);
        assert_eq!(result.max, 42.0);
    }

    #[test]
    fn test_monotonic_increasing() {
        let signal: Vec<_> = (0..100).map(|i| (i as u64, f64::from(i))).collect();
        let rmq = SignalRMQ::new(signal, 32);

        let result = rmq.query_time_range(0, 99).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 99.0);

        let result = rmq.query_time_range(20, 40).unwrap();
        assert_eq!(result.min, 20.0);
        assert_eq!(result.max, 40.0);
    }

    #[test]
    fn test_monotonic_decreasing() {
        let signal: Vec<_> = (0..100).map(|i| (i as u64, f64::from(99 - i))).collect();
        let rmq = SignalRMQ::new(signal, 32);

        let result = rmq.query_time_range(0, 99).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 99.0);

        let result = rmq.query_time_range(20, 40).unwrap();
        assert_eq!(result.min, 59.0);
        assert_eq!(result.max, 79.0);
    }

    #[test]
    fn test_negative_values() {
        let signal = vec![(0, -5.0), (1, 3.0), (2, -2.0), (3, 8.0), (4, -10.0)];
        let rmq = SignalRMQ::new(signal, 2);

        let result = rmq.query_time_range(0, 4).unwrap();
        assert_eq!(result.min, -10.0);
        assert_eq!(result.max, 8.0);

        let result = rmq.query_time_range(0, 2).unwrap();
        assert_eq!(result.min, -5.0);
        assert_eq!(result.max, 3.0);
    }

    #[test]
    fn test_very_large_timestamps() {
        let signal = vec![
            (1_000_000_000, 1.0),
            (2_000_000_000, 5.0),
            (3_000_000_000, 2.0),
            (u64::MAX - 1, 10.0),
        ];
        let rmq = SignalRMQ::new(signal, 2);

        let result = rmq.query_time_range(1_000_000_000, u64::MAX).unwrap();
        assert_eq!(result.min, 1.0);
        assert_eq!(result.max, 10.0);
    }

    #[test]
    fn test_query_before_signal() {
        let signal = vec![(100, 5.0), (200, 10.0)];
        let rmq = SignalRMQ::new(signal, 64);

        assert!(rmq.query_time_range(0, 50).is_none());
        assert!(rmq.query_time_range(0, 99).is_none());
    }

    #[test]
    fn test_query_after_signal() {
        let signal = vec![(100, 5.0), (200, 10.0)];
        let rmq = SignalRMQ::new(signal, 64);

        assert!(rmq.query_time_range(300, 400).is_none());
    }

    #[test]
    fn test_query_partial_overlap_start() {
        let signal = vec![(100, 5.0), (200, 10.0), (300, 15.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(50, 150).unwrap();
        assert_eq!(result.min, 5.0);
        assert_eq!(result.max, 5.0);
    }

    #[test]
    fn test_query_partial_overlap_end() {
        let signal = vec![(100, 5.0), (200, 10.0), (300, 15.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(250, 400).unwrap();
        assert_eq!(result.min, 15.0);
        assert_eq!(result.max, 15.0);
    }

    #[test]
    fn test_query_between_samples() {
        let signal = vec![(100, 5.0), (200, 10.0), (300, 15.0)];
        let rmq = SignalRMQ::new(signal, 64);

        assert!(rmq.query_time_range(110, 190).is_none());
    }

    #[test]
    fn test_small_block_size() {
        let signal: Vec<_> = (0..20).map(|i| (i as u64, f64::from(i))).collect();
        let rmq = SignalRMQ::new(signal, 2);

        let result = rmq.query_time_range(0, 19).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 19.0);

        let result = rmq.query_time_range(5, 15).unwrap();
        assert_eq!(result.min, 5.0);
        assert_eq!(result.max, 15.0);
    }

    #[test]
    fn test_large_block_size() {
        let signal: Vec<_> = (0..20).map(|i| (i as u64, f64::from(i))).collect();
        let rmq = SignalRMQ::new(signal, 100);

        let result = rmq.query_time_range(0, 19).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 19.0);
    }

    #[test]
    fn test_floating_point_precision() {
        let signal = vec![(0, 0.1), (1, 0.2), (2, 0.3)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(0, 2).unwrap();
        assert!((result.min - 0.1).abs() < 1e-10);
        assert!((result.max - 0.3).abs() < 1e-10);
    }

    #[test]
    fn test_special_float_values() {
        let signal = vec![(0, 0.0), (1, -0.0), (2, 1.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(0, 2).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 1.0);
    }

    #[test]
    fn test_query_exact_boundaries() {
        let signal = vec![(10, 1.0), (20, 2.0), (30, 3.0), (40, 4.0)];
        let rmq = SignalRMQ::new(signal, 2);

        // Query exact sample points
        let result = rmq.query_time_range(20, 30).unwrap();
        assert_eq!(result.min, 2.0);
        assert_eq!(result.max, 3.0);
    }

    #[test]
    fn test_index_range_query() {
        let signal: Vec<_> = (0..100).map(|i| (i as u64, f64::from(i))).collect();
        let rmq = SignalRMQ::new(signal, 32);

        let result = rmq.query_index_range(10, 20);
        assert_eq!(result.min, 10.0);
        assert_eq!(result.max, 20.0);

        let result = rmq.query_index_range(0, 99);
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 99.0);
    }

    #[test]
    fn test_nans_in_signal() {
        let mut signal: Vec<_> = (0..100).map(|i| (i + i, i as f64)).collect();
        signal[10].1 = f64::NAN;
        let rmq = SignalRMQ::new(signal, 32);

        // Range containing NaN
        let result = rmq.query_time_range(0, 30).unwrap();
        assert!(result.has_non_finite);

        // Range NOT containing NaN
        let result = rmq.query_time_range(30, 50).unwrap();
        assert!(!result.has_non_finite);
        assert_eq!(result.min, 15.0);
        assert_eq!(result.max, 25.0);
    }

    #[test]
    fn test_all_nans() {
        let signal: Vec<_> = (0..10).map(|i| (i as u64, f64::NAN)).collect();
        let rmq = SignalRMQ::new(signal, 4);

        let result = rmq.query_time_range(0, 9).unwrap();
        assert!(result.has_non_finite);
        // With identity values, min stays INFINITY and max stays NEG_INFINITY
        assert_eq!(result.min, f64::INFINITY);
        assert_eq!(result.max, f64::NEG_INFINITY);
    }

    #[test]
    fn test_nan_propagation() {
        let signal = vec![(0, 1.0), (1, f64::NAN), (2, 3.0)];
        let rmq = SignalRMQ::new(signal, 64);

        // Query including NaN
        let result = rmq.query_time_range(0, 2).unwrap();
        assert!(result.has_non_finite);

        // Non-finite values are excluded from min/max computation.
        // We get the min/max of finite values only.
        assert_eq!(result.min, 1.0);
        assert_eq!(result.max, 3.0);

        // Query excluding NaN
        let result = rmq.query_time_range(2, 2).unwrap();
        assert!(!result.has_non_finite);
        assert_eq!(result.min, 3.0);
    }

    #[test]
    fn test_neg_infinity_values() {
        let signal = vec![(0, 1.0), (1, f64::NEG_INFINITY), (2, 5.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(0, 2).unwrap();
        assert!(result.has_non_finite);
        // NEG_INFINITY is excluded from min/max
        assert_eq!(result.min, 1.0);
        assert_eq!(result.max, 5.0);
    }

    #[test]
    fn test_mixed_non_finite() {
        let signal = vec![
            (0, 1.0),
            (1, f64::NAN),
            (2, 3.0),
            (3, f64::INFINITY),
            (4, 5.0),
            (5, f64::NEG_INFINITY),
            (6, 2.0),
        ];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(0, 6).unwrap();
        assert!(result.has_non_finite);
        // min/max are the extremes of finite values only
        assert_eq!(result.min, 1.0);
        assert_eq!(result.max, 5.0);
    }

    #[test]
    fn test_all_infinity() {
        let signal = vec![
            (0, f64::INFINITY),
            (1, f64::NEG_INFINITY),
            (2, f64::INFINITY),
        ];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_time_range(0, 2).unwrap();
        assert!(result.has_non_finite);
        // No finite values, so min/max remain at identity values
        assert_eq!(result.min, f64::INFINITY);
        assert_eq!(result.max, f64::NEG_INFINITY);
    }

    #[test]
    fn test_single_block_query() {
        let signal: Vec<_> = (0..10).map(|i| (i as u64, f64::from(i))).collect();
        let rmq = SignalRMQ::new(signal, 64);

        // All samples in one block
        let result = rmq.query_time_range(0, 9).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 9.0);
    }

    #[test]
    fn test_cross_block_boundary() {
        let signal: Vec<_> = (0..100).map(|i| (i as u64, f64::from(i))).collect();
        let rmq = SignalRMQ::new(signal, 32);

        // Query that crosses block boundary at 32
        let result = rmq.query_time_range(30, 35).unwrap();
        assert_eq!(result.min, 30.0);
        assert_eq!(result.max, 35.0);
    }

    #[test]
    fn test_zigzag_pattern() {
        let mut signal = Vec::new();
        for i in 0..100 {
            let value = if i % 2 == 0 { 0.0 } else { 100.0 };
            signal.push((i as u64, value));
        }
        let rmq = SignalRMQ::new(signal, 16);

        let result = rmq.query_time_range(0, 99).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 100.0);

        // Any subrange should also contain both extremes
        let result = rmq.query_time_range(10, 20).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 100.0);
    }

    #[test]
    fn test_parse_numeric_value() {
        // Hex
        assert_eq!(parse_numeric_value("f9", "Hex"), Some(249.0));
        assert_eq!(parse_numeric_value("ca", "Hexadecimal"), Some(202.0));
        assert_eq!(parse_numeric_value("80", "Hex"), Some(128.0));
        assert_eq!(parse_numeric_value("10", "Hex"), Some(16.0));
        assert_eq!(parse_numeric_value("0x10", "Hex"), Some(16.0));
        assert_eq!(parse_numeric_value("0xFF", "Hexadecimal"), Some(255.0));

        // Decimal
        assert_eq!(parse_numeric_value("123", "Unsigned"), Some(123.0));
        assert_eq!(parse_numeric_value("123.45", "Float"), Some(123.45));
        assert_eq!(parse_numeric_value("80", "Unsigned"), Some(80.0));
        assert_eq!(parse_numeric_value("10", "Signed"), Some(10.0));
        assert_eq!(parse_numeric_value("1.5e3", "Float"), Some(1500.0));
        assert_eq!(parse_numeric_value("-3.14e-2", "Float"), Some(-0.0314));

        // Binary
        assert_eq!(parse_numeric_value("1010", "Binary"), Some(10.0));
        assert_eq!(parse_numeric_value("0b1010", "Binary"), Some(10.0));
        assert_eq!(parse_numeric_value("11111111", "Bin"), Some(255.0));

        // Fallback to hex for non-decimal strings
        assert_eq!(parse_numeric_value("f9", "Unsigned"), Some(249.0));
        assert_eq!(parse_numeric_value("ca", "Signed"), Some(202.0));

        // Invalid
        assert_eq!(parse_numeric_value("xyz", "Hex"), None);
        assert_eq!(parse_numeric_value("invalid", "Unsigned"), None);
        assert_eq!(parse_numeric_value("12", "Binary"), None);

        // 128-bit max value in hex: 2^128 - 1 = 340282366920938463463374607431768211455
        let hex_128bit = "ffffffffffffffffffffffffffffffff";
        let result = parse_numeric_value(hex_128bit, "Hexadecimal");
        assert_eq!(result, Some(3.402823669209385e38));

        // 256-bit max value in hex: 2^256 - 1
        let hex_256bit = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let result = parse_numeric_value(hex_256bit, "Hex");
        assert_eq!(result, Some(1.157920892373162e77));

        // Large binary value (128 bits): same as hex_128bit
        let bin_128bit = "11111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111";
        let result = parse_numeric_value(bin_128bit, "Binary");
        assert_eq!(result, Some(3.402823669209385e38));

        // 64-bit max value in hex: 2^64 - 1 = 18446744073709551615
        let hex_64bit = "ffffffffffffffff";
        let result = parse_numeric_value(hex_64bit, "Hexadecimal");
        assert_eq!(result, Some(1.8446744073709552e19));

        // Hex and Unsigned should produce same result for same numeric value
        // 128-bit value as decimal string (from Unsigned translator)
        let decimal_128bit = "340282366920938463463374607431768211455";
        let result = parse_numeric_value(decimal_128bit, "Unsigned");
        assert_eq!(result, Some(3.402823669209385e38));
    }

    #[test]
    fn test_query_at_time_exact_match() {
        let signal = vec![(10, 1.0), (20, 2.0), (30, 3.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_at_time(20);
        assert_eq!(result.current, Some((20, 2.0)));
        assert_eq!(result.next, Some(30));
    }

    #[test]
    fn test_query_at_time_between_samples() {
        let signal = vec![(10, 1.0), (20, 2.0), (30, 3.0)];
        let rmq = SignalRMQ::new(signal, 64);

        // Query at time 15 (between 10 and 20)
        let result = rmq.query_at_time(15);
        assert_eq!(result.current, Some((10, 1.0)));
        assert_eq!(result.next, Some(20));

        // Query at time 25 (between 20 and 30)
        let result = rmq.query_at_time(25);
        assert_eq!(result.current, Some((20, 2.0)));
        assert_eq!(result.next, Some(30));
    }

    #[test]
    fn test_query_at_time_before_first_sample() {
        let signal = vec![(10, 1.0), (20, 2.0), (30, 3.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_at_time(5);
        assert_eq!(result.current, None);
        assert_eq!(result.next, Some(10));
    }

    #[test]
    fn test_query_at_time_after_last_sample() {
        let signal = vec![(10, 1.0), (20, 2.0), (30, 3.0)];
        let rmq = SignalRMQ::new(signal, 64);

        let result = rmq.query_at_time(40);
        assert_eq!(result.current, Some((30, 3.0)));
        assert_eq!(result.next, None);
    }

    #[test]
    fn test_query_at_time_single_sample() {
        let signal = vec![(100, 5.0)];
        let rmq = SignalRMQ::new(signal, 64);

        // Before
        let result = rmq.query_at_time(50);
        assert_eq!(result.current, None);
        assert_eq!(result.next, Some(100));

        // Exact
        let result = rmq.query_at_time(100);
        assert_eq!(result.current, Some((100, 5.0)));
        assert_eq!(result.next, None);

        // After
        let result = rmq.query_at_time(200);
        assert_eq!(result.current, Some((100, 5.0)));
        assert_eq!(result.next, None);
    }

    #[test]
    fn test_query_at_time_at_boundaries() {
        let signal = vec![(0, 1.0), (100, 2.0)];
        let rmq = SignalRMQ::new(signal, 64);

        // At first sample
        let result = rmq.query_at_time(0);
        assert_eq!(result.current, Some((0, 1.0)));
        assert_eq!(result.next, Some(100));

        // At last sample
        let result = rmq.query_at_time(100);
        assert_eq!(result.current, Some((100, 2.0)));
        assert_eq!(result.next, None);
    }
}
