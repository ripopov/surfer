pub mod event_table;
pub mod konata_events;
pub mod konata_instructions;
pub mod konata_statistics;
pub mod multi_signal_change_list;
pub mod multi_signal_index;
pub mod signal_analysis;
pub mod signal_change_list;
mod signal_formatting;
pub mod transaction_trace;
pub mod virtual_model;

pub use event_table::EventTableModel;
pub use konata_events::KonataEventTableModel;
pub use konata_instructions::KonataInstructionTableModel;
pub use konata_statistics::{KonataStatisticsInput, KonataStatisticsTableModel};
pub use multi_signal_change_list::{MultiSignalChangeListModel, decode_signal_column_key};
pub use multi_signal_index::{
    MergedIndex, SignalRuns, TransitionAtTime, dedup_multi_signal_entries,
};
pub use signal_analysis::{
    SignalAnalysisAccumulation, SignalAnalysisInterval, SignalAnalysisMarker,
    SignalAnalysisMetrics, SignalAnalysisResultsModel, SignalAnalysisTimeRange,
    accumulate_signal_metrics, build_intervals, collect_trigger_times, infer_sampling_mode,
    interval_index_for_time, normalize_markers, normalize_time_range,
};
pub use signal_change_list::SignalChangeListModel;
pub use transaction_trace::TransactionTraceModelWithData;
pub use virtual_model::VirtualTableModel;
