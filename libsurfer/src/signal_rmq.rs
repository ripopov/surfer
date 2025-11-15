/// Fast min/max queries on time-indexed signals using Blocked RMQ + Binary Search
///
/// RMQ = Range Minimum/Maximum Query
///
/// # Algorithm
///
/// 1. **Binary Search on Timestamps**: Convert time range [t_start, t_end] to array index range [L, R]
///    in O(log N) time. This works regardless of irregular timestamp spacing.
///
/// 2. **Blocked RMQ Structure**:
///    - Split signal into fixed-size blocks (typically 32 or 64 samples)
///    - Precompute min/max for each block
///    - Build a sparse table over block summaries for O(1) queries across multiple blocks
///
/// 3. **Query Execution**:
///    - Handle partial blocks at boundaries directly
///    - Use sparse table for complete blocks in the middle
///    - Combine results in O(1) time
///
/// # Complexity
///
/// - Construction: O(N + N/B · log(N/B))
/// - Time queries: O(log N) for binary search + O(1) for min/max
/// - Memory: O(N + N/B · log(N/B)) where B is the block size
///
/// This approach is 10-30× more memory efficient than full sparse table RMQ
/// while maintaining excellent query performance.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinMax {
    pub min: f64,
    pub max: f64,
    pub has_nan: bool,
}

impl MinMax {
    fn new(value: f64) -> Self {
        Self {
            min: value,
            max: value,
            has_nan: value.is_nan(),
        }
    }

    fn combine(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            has_nan: self.has_nan || other.has_nan,
        }
    }

    fn from_slice(values: &[f64]) -> Self {
        values.iter().fold(Self::new(values[0]), |acc, &v| Self {
            min: acc.min.min(v),
            max: acc.max.max(v),
            has_nan: acc.has_nan || v.is_nan(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct SignalRMQ {
    /// Sorted timestamps
    timestamps: Vec<u64>,
    /// Signal values corresponding to timestamps
    values: Vec<f64>,
    /// Block size (typically 32 or 64)
    block_size: usize,
    /// Min/Max for each block
    block_summaries: Vec<MinMax>,
    /// Sparse table over block summaries
    /// sparse_table[level][block_idx] contains min/max for 2^level blocks starting at block_idx
    sparse_table: Vec<Vec<MinMax>>,
}

impl SignalRMQ {
    /// Create a new SignalRMQ from an iterator of (timestamp, value) pairs
    ///
    /// # Arguments
    /// * `signal` - Iterator over (timestamp, value) pairs (must be sorted by timestamp)
    /// * `block_size` - Size of each block (default: 64)
    ///
    /// # Panics
    /// Panics if the signal is empty or timestamps are not strictly increasing
    pub fn new<I>(signal: I, block_size: usize) -> Self
    where
        I: IntoIterator<Item = (u64, f64)>,
    {
        let data: Vec<(u64, f64)> = signal.into_iter().collect();

        assert!(!data.is_empty(), "Signal cannot be empty");

        // Verify timestamps are strictly increasing
        for i in 1..data.len() {
            assert!(
                data[i].0 > data[i - 1].0,
                "Timestamps must be strictly increasing"
            );
        }

        let timestamps: Vec<u64> = data.iter().map(|&(t, _)| t).collect();
        let values: Vec<f64> = data.iter().map(|&(_, v)| v).collect();

        let n = values.len();
        let num_blocks = (n + block_size - 1) / block_size;

        // Build block summaries
        let mut block_summaries = Vec::with_capacity(num_blocks);
        for block_idx in 0..num_blocks {
            let start = block_idx * block_size;
            let end = (start + block_size).min(n);
            block_summaries.push(MinMax::from_slice(&values[start..end]));
        }

        // Build sparse table over blocks
        let sparse_table = Self::build_sparse_table(&block_summaries);

        Self {
            timestamps,
            values,
            block_size,
            block_summaries,
            sparse_table,
        }
    }

    /// Build a sparse table over block summaries
    fn build_sparse_table(block_summaries: &[MinMax]) -> Vec<Vec<MinMax>> {
        let num_blocks = block_summaries.len();
        if num_blocks == 0 {
            return vec![];
        }

        let max_level = if num_blocks == 1 {
            1
        } else {
            (num_blocks as f64).log2().floor() as usize + 1
        };

        let mut table = Vec::with_capacity(max_level);

        // Level 0: individual blocks
        table.push(block_summaries.to_vec());

        // Build higher levels
        for level in 1..max_level {
            let prev_level = &table[level - 1];
            let jump = 1 << level;
            let mut current_level = Vec::new();

            for i in 0..num_blocks {
                if i + jump / 2 < num_blocks {
                    let combined = prev_level[i].combine(&prev_level[i + jump / 2]);
                    current_level.push(combined);
                } else {
                    current_level.push(prev_level[i]);
                }
            }

            table.push(current_level);
        }

        table
    }

    /// Query min/max values in the time range [t_start, t_end] (inclusive)
    ///
    /// Returns None if the time range doesn't intersect with the signal
    pub fn query_time_range(&self, t_start: u64, t_end: u64) -> Option<MinMax> {
        if t_start > t_end {
            return None;
        }

        // Binary search to find index range
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

    /// Query min/max values in the index range [l, r] (inclusive)
    pub fn query_index_range(&self, l: usize, r: usize) -> MinMax {
        assert!(l <= r && r < self.values.len(), "Invalid index range");

        let l_block = l / self.block_size;
        let r_block = r / self.block_size;

        // Case 1: Query is within a single block
        if l_block == r_block {
            return MinMax::from_slice(&self.values[l..=r]);
        }

        // Case 2: Query spans multiple blocks
        let mut result = MinMax::new(self.values[l]);

        // Left partial block
        let l_block_end = (l_block + 1) * self.block_size - 1;
        if l <= l_block_end {
            let partial = MinMax::from_slice(&self.values[l..=(l_block_end.min(r))]);
            result = result.combine(&partial);
        }

        // Right partial block
        let r_block_start = r_block * self.block_size;
        if r_block > l_block && r_block_start <= r {
            let partial = MinMax::from_slice(&self.values[r_block_start..=r]);
            result = result.combine(&partial);
        }

        // Middle complete blocks using sparse table
        let first_full_block = l_block + 1;
        let last_full_block = if r_block_start <= r {
            r_block - 1
        } else {
            r_block
        };

        if first_full_block <= last_full_block {
            let middle = self.query_blocks(first_full_block, last_full_block);
            result = result.combine(&middle);
        }

        result
    }

    /// Query min/max over a range of complete blocks using the sparse table
    fn query_blocks(&self, l_block: usize, r_block: usize) -> MinMax {
        if l_block > r_block {
            return MinMax::new(self.values[0]); // Should not happen
        }

        if l_block == r_block {
            return self.block_summaries[l_block];
        }

        // Find the largest power of 2 that fits in the range
        let range_len = r_block - l_block + 1;
        let level = (range_len as f64).log2().floor() as usize;
        let jump = 1 << level;

        // Combine two overlapping ranges
        let left = self.sparse_table[level][l_block];
        let right = self.sparse_table[level][r_block - jump + 1];

        left.combine(&right)
    }

    /// Get the number of samples in the signal
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if the signal is empty
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get the time range covered by the signal
    pub fn time_range(&self) -> Option<(u64, u64)> {
        if self.timestamps.is_empty() {
            None
        } else {
            Some((self.timestamps[0], *self.timestamps.last().unwrap()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_sample() {
        let signal = vec![(100, 5.0)];
        let rmq = SignalRMQ::new(signal, 64);

        assert_eq!(rmq.len(), 1);
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
            signal.push((i as u64, (i % 100) as f64));
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
        let signal: Vec<_> = (0..100).map(|i| (i as u64, i as f64)).collect();
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
        let signal: Vec<_> = (0..100).map(|i| (i as u64, (99 - i) as f64)).collect();
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
        let signal: Vec<_> = (0..20).map(|i| (i as u64, i as f64)).collect();
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
        let signal: Vec<_> = (0..20).map(|i| (i as u64, i as f64)).collect();
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
    #[should_panic(expected = "Signal cannot be empty")]
    fn test_empty_signal() {
        let signal: Vec<(u64, f64)> = vec![];
        SignalRMQ::new(signal, 64);
    }

    #[test]
    #[should_panic(expected = "Timestamps must be strictly increasing")]
    fn test_duplicate_timestamps() {
        let signal = vec![(100, 5.0), (100, 10.0)];
        SignalRMQ::new(signal, 64);
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
        let signal: Vec<_> = (0..100).map(|i| (i as u64, i as f64)).collect();
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
        let mut signal: Vec<_> = (0..100).map(|i| (i + i as u64, i as f64)).collect();
        signal[10].1 = f64::NAN;
        let rmq = SignalRMQ::new(signal, 32);

        // Range containing NaN
        let result = rmq.query_time_range(0, 30).unwrap();
        assert!(result.has_nan);

        // Range NOT containing NaN
        let result = rmq.query_time_range(30, 50).unwrap();
        assert!(!result.has_nan);
        assert_eq!(result.min, 15.0);
        assert_eq!(result.max, 25.0);
    }

    #[test]
    fn test_all_nans() {
        let signal: Vec<_> = (0..10).map(|i| (i as u64, f64::NAN)).collect();
        let rmq = SignalRMQ::new(signal, 4);

        let result = rmq.query_time_range(0, 9).unwrap();
        assert!(result.has_nan);
        assert!(result.min.is_nan());
        assert!(result.max.is_nan());
    }

    #[test]
    fn test_nan_propagation() {
        let signal = vec![(0, 1.0), (1, f64::NAN), (2, 3.0)];
        let rmq = SignalRMQ::new(signal, 64);

        // Query including NaN
        let result = rmq.query_time_range(0, 2).unwrap();
        assert!(result.has_nan);

        // Rust's f64::min/max ignores NaN if the other value is not NaN.
        // This is desirable behavior: we get the min/max of valid numbers,
        // but we also know there was a NaN via has_nan.
        assert_eq!(result.min, 1.0);
        assert_eq!(result.max, 3.0);

        // Query excluding NaN
        let result = rmq.query_time_range(2, 2).unwrap();
        assert!(!result.has_nan);
        assert_eq!(result.min, 3.0);
    }

    #[test]
    fn test_single_block_query() {
        let signal: Vec<_> = (0..10).map(|i| (i as u64, i as f64)).collect();
        let rmq = SignalRMQ::new(signal, 64);

        // All samples in one block
        let result = rmq.query_time_range(0, 9).unwrap();
        assert_eq!(result.min, 0.0);
        assert_eq!(result.max, 9.0);
    }

    #[test]
    fn test_cross_block_boundary() {
        let signal: Vec<_> = (0..100).map(|i| (i as u64, i as f64)).collect();
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
}
