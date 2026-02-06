pub mod multi_signal_change_list;
pub mod multi_signal_index;
pub mod signal_analysis;
pub mod signal_change_list;
pub mod transaction_trace;
pub mod virtual_model;

pub use multi_signal_change_list::{MultiSignalChangeListModel, decode_signal_column_key};
pub use multi_signal_index::{
    MergedIndex, SignalRuns, TransitionAtTime, dedup_multi_signal_entries,
};
pub use signal_analysis::{
    SignalAnalysisAccumulation, SignalAnalysisInterval, SignalAnalysisMarker,
    SignalAnalysisMetrics, SignalAnalysisTimeRange, accumulate_signal_metrics, build_intervals,
    collect_trigger_times, infer_sampling_mode, interval_index_for_time, normalize_markers,
    normalize_time_range,
};
pub use signal_change_list::SignalChangeListModel;
pub use transaction_trace::TransactionTraceModelWithData;
pub use virtual_model::VirtualTableModel;
