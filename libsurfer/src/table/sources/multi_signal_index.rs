use crate::table::{MultiSignalEntry, TableRowId};
use std::collections::{HashMap, HashSet};

/// Compressed transition run metadata for a unique signal timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionAtTime {
    pub time_u64: u64,
    pub run_start: u32,
    pub run_len: u16,
}

/// Sparse per-signal transition runs keyed by unique timestamp.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignalRuns {
    pub transitions: Vec<TransitionAtTime>,
}

impl SignalRuns {
    /// Build compressed transition runs from a signal transition time stream.
    pub fn from_transition_times<I>(times: I) -> Self
    where
        I: IntoIterator<Item = u64>,
    {
        let mut transitions: Vec<TransitionAtTime> = Vec::new();
        for (idx, time_u64) in times.into_iter().enumerate() {
            if let Some(last) = transitions.last_mut()
                && last.time_u64 == time_u64
            {
                last.run_len = last.run_len.saturating_add(1);
                continue;
            }

            transitions.push(TransitionAtTime {
                time_u64,
                run_start: saturating_u32(idx),
                run_len: 1,
            });
        }

        Self { transitions }
    }

    /// Exact lookup for a transition run at `time_u64`.
    #[must_use]
    pub fn exact_run(&self, time_u64: u64) -> Option<&TransitionAtTime> {
        self.transitions
            .binary_search_by_key(&time_u64, |run| run.time_u64)
            .ok()
            .and_then(|idx| self.transitions.get(idx))
    }

    /// Strict previous lookup for transition run before `time_u64`.
    #[must_use]
    pub fn previous_run(&self, time_u64: u64) -> Option<&TransitionAtTime> {
        let idx = match self
            .transitions
            .binary_search_by_key(&time_u64, |run| run.time_u64)
        {
            Ok(idx) => idx.checked_sub(1)?,
            Err(0) => return None,
            Err(insert_idx) => insert_idx - 1,
        };

        self.transitions.get(idx)
    }
}

/// Sparse merged timeline index across all selected signals.
#[derive(Debug, Clone, Default)]
pub struct MergedIndex {
    pub row_times: Vec<u64>,
    pub row_ids: Vec<TableRowId>,
    pub row_index: HashMap<TableRowId, usize>,
    pub signal_time_runs: Vec<SignalRuns>,
}

impl MergedIndex {
    /// Build merged timeline index from transition iterators.
    ///
    /// Per signal iterator items are expected to be transition pairs `(time_u64, value)`.
    /// Values are ignored by this stage's pure index builder.
    pub fn from_transition_iters<I, S, V>(signals: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: IntoIterator<Item = (u64, V)>,
    {
        let mut all_times = HashSet::new();
        let mut signal_time_runs = Vec::new();

        for signal in signals {
            let mut transition_times = Vec::new();
            for (time_u64, _value) in signal {
                transition_times.push(time_u64);
                all_times.insert(time_u64);
            }
            signal_time_runs.push(SignalRuns::from_transition_times(transition_times));
        }

        let mut row_times: Vec<u64> = all_times.into_iter().collect();
        row_times.sort_unstable();

        let mut row_ids = Vec::with_capacity(row_times.len());
        let mut row_index = HashMap::with_capacity(row_times.len());
        for (idx, time_u64) in row_times.iter().copied().enumerate() {
            let row_id = TableRowId(time_u64);
            row_ids.push(row_id);
            row_index.insert(row_id, idx);
        }

        Self {
            row_times,
            row_ids,
            row_index,
            signal_time_runs,
        }
    }

    /// Build merged timeline index from transition-time iterators.
    pub fn from_transition_time_iters<I, S>(signals: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: IntoIterator<Item = u64>,
    {
        Self::from_transition_iters(
            signals
                .into_iter()
                .map(|signal| signal.into_iter().map(|time_u64| (time_u64, ()))),
        )
    }

    #[must_use]
    pub fn exact_run(&self, signal_idx: usize, time_u64: u64) -> Option<&TransitionAtTime> {
        self.signal_time_runs.get(signal_idx)?.exact_run(time_u64)
    }

    #[must_use]
    pub fn previous_run(&self, signal_idx: usize, time_u64: u64) -> Option<&TransitionAtTime> {
        self.signal_time_runs
            .get(signal_idx)?
            .previous_run(time_u64)
    }
}

/// Deduplicate selected signal entries by `(VariableRef, field)` preserving first occurrence.
pub fn dedup_multi_signal_entries<I>(entries: I) -> Vec<MultiSignalEntry>
where
    I: IntoIterator<Item = MultiSignalEntry>,
{
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for entry in entries {
        let key = (entry.variable.clone(), entry.field.clone());
        if seen.insert(key) {
            deduped.push(entry);
        }
    }

    deduped
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{MergedIndex, SignalRuns, TransitionAtTime, dedup_multi_signal_entries};
    use crate::table::{MultiSignalEntry, TableRowId};
    use crate::wave_container::{VariableRef, VariableRefExt};

    #[test]
    fn merged_index_dedups_and_sorts_global_timeline() {
        let index = MergedIndex::from_transition_time_iters([
            vec![10, 20, 20, 30],
            vec![5, 20, 25],
            Vec::new(),
        ]);

        assert_eq!(index.row_times, vec![5, 10, 20, 25, 30]);
        assert_eq!(
            index.row_ids,
            vec![
                TableRowId(5),
                TableRowId(10),
                TableRowId(20),
                TableRowId(25),
                TableRowId(30),
            ]
        );
        assert_eq!(index.row_index.get(&TableRowId(5)), Some(&0));
        assert_eq!(index.row_index.get(&TableRowId(10)), Some(&1));
        assert_eq!(index.row_index.get(&TableRowId(20)), Some(&2));
        assert_eq!(index.row_index.get(&TableRowId(25)), Some(&3));
        assert_eq!(index.row_index.get(&TableRowId(30)), Some(&4));
        assert_eq!(index.signal_time_runs.len(), 3);
    }

    #[test]
    fn signal_runs_group_same_timestamp_transitions() {
        let runs = SignalRuns::from_transition_times([5, 5, 5, 10, 10, 12]);

        assert_eq!(
            runs.transitions,
            vec![
                TransitionAtTime {
                    time_u64: 5,
                    run_start: 0,
                    run_len: 3,
                },
                TransitionAtTime {
                    time_u64: 10,
                    run_start: 3,
                    run_len: 2,
                },
                TransitionAtTime {
                    time_u64: 12,
                    run_start: 5,
                    run_len: 1,
                },
            ]
        );
    }

    #[test]
    fn signal_runs_exact_and_previous_lookup_are_logarithmic_and_correct() {
        let runs = SignalRuns::from_transition_times([10, 10, 20, 40, 40, 40]);

        assert_eq!(
            runs.exact_run(10),
            Some(&TransitionAtTime {
                time_u64: 10,
                run_start: 0,
                run_len: 2,
            })
        );
        assert_eq!(runs.exact_run(15), None);

        assert_eq!(runs.previous_run(5), None);
        assert_eq!(runs.previous_run(10), None);
        assert_eq!(
            runs.previous_run(11),
            Some(&TransitionAtTime {
                time_u64: 10,
                run_start: 0,
                run_len: 2,
            })
        );
        assert_eq!(
            runs.previous_run(20),
            Some(&TransitionAtTime {
                time_u64: 10,
                run_start: 0,
                run_len: 2,
            })
        );
        assert_eq!(
            runs.previous_run(41),
            Some(&TransitionAtTime {
                time_u64: 40,
                run_start: 3,
                run_len: 3,
            })
        );
    }

    #[test]
    fn merged_index_exact_and_previous_lookup_route_to_signal_runs() {
        let index = MergedIndex::from_transition_time_iters([vec![10, 10, 20], vec![7, 9, 9]]);

        assert_eq!(
            index.exact_run(0, 10),
            Some(&TransitionAtTime {
                time_u64: 10,
                run_start: 0,
                run_len: 2,
            })
        );
        assert_eq!(
            index.previous_run(1, 10),
            Some(&TransitionAtTime {
                time_u64: 9,
                run_start: 1,
                run_len: 2,
            })
        );
        assert_eq!(index.exact_run(99, 10), None);
    }

    #[test]
    fn dedup_multi_signal_entries_by_variable_and_field() {
        let clk = VariableRef::from_hierarchy_string("tb.clk");
        let counter = VariableRef::from_hierarchy_string("tb.dut.counter");

        let deduped = dedup_multi_signal_entries(vec![
            MultiSignalEntry {
                variable: clk.clone(),
                field: vec![],
            },
            MultiSignalEntry {
                variable: counter.clone(),
                field: vec!["value".to_string()],
            },
            MultiSignalEntry {
                variable: clk.clone(),
                field: vec![],
            },
            MultiSignalEntry {
                variable: counter.clone(),
                field: vec!["value".to_string()],
            },
            MultiSignalEntry {
                variable: counter.clone(),
                field: vec!["next".to_string()],
            },
        ]);

        assert_eq!(deduped.len(), 3);
        assert_eq!(deduped[0].variable, clk);
        assert!(deduped[0].field.is_empty());
        assert_eq!(deduped[1].variable, counter);
        assert_eq!(deduped[1].field, vec!["value".to_string()]);
        assert_eq!(deduped[2].variable, counter);
        assert_eq!(deduped[2].field, vec!["next".to_string()]);
    }
}
