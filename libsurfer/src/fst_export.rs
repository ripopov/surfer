//! Exporting the currently displayed variables to an FST waveform file.
//!
//! Only the displayed variables (and only the hierarchy needed to reach them, not the full
//! design hierarchy) are written out.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use ahash::{AHashMap, AHashSet};
use camino::{Utf8Path, Utf8PathBuf};
use eyre::Result;
use fst_writer::{
    FstBodyWriter, FstFileType, FstHeaderWriter, FstInfo, FstScopeType, FstSignalId, FstSignalType,
    FstVarDirection, FstVarType, open_fst,
};
use rfd::FileHandle;
use tracing::{error, warn};
use wellen::{
    Hierarchy, ScopeRef, ScopeType, Signal, SignalEncoding, SignalRef, SignalValueRef, Time,
    TimeTable, TimeTableIdx, Timescale, TimescaleUnit, VarDirection, VarRef, VarType,
};

use crate::SystemState;
use crate::async_util::{AsyncJob, perform_async_work};
use crate::channels::checked_send_many;
use crate::displayed_item::DisplayedItem;
use crate::file_dialog::FST_EXPORT_FILTER;
use crate::message::Message;
use crate::state_file_io::{sanitize_file_stem, source_file_stem};
use crate::time::{TimeScale, TimeUnit};
use crate::wave_container::VariableRef;
use crate::wellen::WellenContainer;

impl SystemState {
    /// Variables backing the currently displayed variables, in display order and without
    /// duplicates.
    fn displayed_variables(&self) -> Vec<VariableRef> {
        let Some(waves) = self.user.waves.as_ref() else {
            return vec![];
        };
        waves
            .items_tree
            .iter()
            .filter_map(|node| match waves.displayed_items.get(&node.item_ref) {
                Some(DisplayedItem::Variable(v)) => Some(v.variable_ref.clone()),
                _ => None,
            })
            .collect()
    }

    /// Builds the suggested FST export file name, based on the loaded wave source's
    /// name when available.
    fn default_fst_export_file_name(&self) -> String {
        let stem = self
            .user
            .waves
            .as_ref()
            .and_then(|waves| source_file_stem(&waves.source))
            .map(|stem| sanitize_file_stem(stem, "export"));
        match stem {
            Some(stem) => format!("{stem}_export.fst"),
            None => "export.fst".to_string(),
        }
    }

    /// Exports the currently displayed variables to an FST file.
    ///
    /// When `path` is `None`, this opens a save dialog with a suggested filename.
    pub(crate) fn export_signals_to_fst(&mut self, path: Option<Utf8PathBuf>) {
        let variables = self.displayed_variables();
        if variables.is_empty() {
            error!("No displayed variables to export to FST");
            return;
        }
        let Some(waves) = self.user.waves.as_ref() else {
            return;
        };
        let export = match waves
            .inner
            .as_waves()
            .ok_or_else(|| eyre::eyre!("No waveform data to export"))
            .and_then(|w| w.prepare_fst_export(&variables))
        {
            Ok(export) => export,
            Err(e) => {
                error!("Failed to prepare FST export: {e:#?}");
                return;
            }
        };

        let messages = move |destination: FileHandle| async move {
            let result = match Utf8Path::from_path(destination.path()) {
                Some(path) => export.write_to_file(path),
                None => Err(eyre::eyre!(
                    "File path '{}' contains invalid UTF-8",
                    destination.path().display()
                )),
            };
            match result {
                Ok(()) => vec![Message::AsyncDone(AsyncJob::ExportFst)],
                Err(e) => {
                    error!("Failed to export variables to FST: {e:#?}");
                    vec![Message::Error(e), Message::AsyncDone(AsyncJob::ExportFst)]
                }
            }
        };

        if let Some(path) = path {
            let sender = self.channels.msg_sender.clone();
            perform_async_work(async move {
                checked_send_many(&sender, messages(path.into_std_path_buf().into()).await);
            });
        } else {
            self.file_dialog_save(
                "Export variables to FST",
                &FST_EXPORT_FILTER,
                Some(self.default_fst_export_file_name()),
                messages,
            );
        }
    }
}

/// Backend-agnostic, owned snapshot of the data needed to write signals out to an FST file.
pub(crate) enum FstExport {
    Wellen(FstExportData),
}

impl FstExport {
    /// Writes the previously selected variables to an FST file at `path`.
    pub(crate) fn write_to_file(&self, path: &Utf8Path) -> Result<()> {
        match self {
            FstExport::Wellen(data) => data.write_to_file(path),
        }
    }
}

/// A single node of the minimal scope tree needed to export a set of variables to FST.
///
/// Only scopes that lie on the path to an exported variable are included, and only the
/// exported variables themselves are attached, not their siblings.
#[derive(Default)]
pub(crate) struct FstExportNode {
    /// Wellen scope id, if this node corresponds to an actual scope in the hierarchy.
    pub(crate) id: Option<ScopeRef>,
    /// Child scopes, keyed by name.
    pub(crate) children: BTreeMap<String, FstExportNode>,
    /// Variables exported directly under this scope.
    pub(crate) vars: Vec<VarRef>,
}

/// A self-contained snapshot of everything needed to write a set of variables (and only
/// the hierarchy needed to reach them) to an FST file.
///
/// Kept separate from `WellenContainer` so that the actual (potentially slow) file
/// writing can be handed off to a background task without holding a borrow of it.
pub(crate) struct FstExportData {
    /// Full source hierarchy, needed to resolve scope/var metadata when writing.
    pub(crate) hierarchy: Arc<Hierarchy>,
    /// Full source time table; only entries referenced by the exported signals are used.
    pub(crate) time_table: Arc<TimeTable>,
    /// Minimal scope tree containing only the exported variables.
    pub(crate) root: FstExportNode,
    /// Only the signals that are actually referenced by `root`.
    pub(crate) signals: AHashMap<SignalRef, Arc<Signal>>,
    /// FST header version string, copied from the source hierarchy.
    pub(crate) version: String,
    /// FST header date string, copied from the source hierarchy.
    pub(crate) date: String,
    /// Final FST timescale exponent: the source unit's exponent plus the multiplier's
    /// and time stamps' folded-in digits.
    pub(crate) timescale_exponent: i8,
    /// `10.pow(time_scale_shift)`, precomputed so [`scale_time`] doesn't need to redo it
    /// for every time stamp. See [`common_power_of_ten`].
    pub(crate) time_scale_divisor: u128,
    /// Scaled first time stamp of the source simulation, written into the FST header.
    pub(crate) start_time: Time,
    /// The last time stamp of the source simulation, so the export can cover the same
    /// time span even if no exported signal changes at that point.
    pub(crate) end_time: Time,
}

impl FstExportData {
    /// Writes the previously selected variables to an FST file at `path`.
    pub(crate) fn write_to_file(&self, path: &Utf8Path) -> Result<()> {
        let h: &Hierarchy = &self.hierarchy;
        let info = FstInfo {
            start_time: self.start_time,
            timescale_exponent: self.timescale_exponent,
            version: self.version.clone(),
            date: self.date.clone(),
            file_type: FstFileType::VerilogVhdl,
        };

        let mut header = open_fst(path, &info)?;
        let mut signal_ids: AHashMap<SignalRef, FstSignalId> = AHashMap::new();
        write_fst_export_node(h, &mut header, &self.root, &mut signal_ids)?;
        let mut body = header.finish()?;

        // Collect the actual signal data (deduplicated) so that changes can be merged in time order.
        let signals = signal_ids
            .iter()
            .map(|(signal_ref, fst_id)| (*fst_id, Arc::clone(&self.signals[signal_ref])))
            .collect::<Vec<_>>();

        self.write_fst_value_changes(&mut body, &signals)?;
        body.finish()?;
        Ok(())
    }

    /// Merges the value changes of `signals` in time order and writes them to `out`, rescaling
    /// each time stamp with [`scale_time`] (see `time_scale_divisor`).
    fn write_fst_value_changes<W: std::io::Write + std::io::Seek>(
        &self,
        out: &mut FstBodyWriter<W>,
        signals: &[(FstSignalId, Arc<Signal>)],
    ) -> Result<()> {
        let mut iters = signals
            .iter()
            .map(|(fst_id, signal)| (*fst_id, signal.iter_changes().peekable()))
            .collect::<Vec<_>>();

        let mut heap = BinaryHeap::new();
        for (i, (_, it)) in iters.iter_mut().enumerate() {
            if let Some((t, _)) = it.peek() {
                heap.push(Reverse((*t, i)));
            }
        }

        let mut last_time_idx: Option<TimeTableIdx> = None;
        while let Some(Reverse((time_idx, i))) = heap.pop() {
            if last_time_idx != Some(time_idx) {
                let time = self.time_table.get(time_idx as usize).copied().unwrap_or(0);
                out.time_change(scale_time(time, self.time_scale_divisor))?;
                last_time_idx = Some(time_idx);
            }

            let (fst_id, it) = &mut iters[i];
            let (_, value) = it.next().expect("time index was just peeked");
            match value {
                SignalValueRef::Event => out.signal_change(*fst_id, &[])?,
                SignalValueRef::BitVec(bv) => {
                    out.signal_change(*fst_id, bv.bit_string().as_bytes())?;
                }
                SignalValueRef::Real(v) => out.signal_change(*fst_id, &v.to_le_bytes())?,
                SignalValueRef::String(_) => {
                    // variable-length strings are not declared during export, so this should not occur
                }
            }

            if let Some((next_idx, _)) = it.peek() {
                heap.push(Reverse((*next_idx, i)));
            }
        }

        // Emit a final time change even if no exported signal changes there, so the export
        // covers the same time span as the source.
        let last_idx = self
            .time_table
            .len()
            .checked_sub(1)
            .map(|idx| idx as TimeTableIdx);
        if !self.time_table.is_empty() && last_time_idx != last_idx {
            out.time_change(self.end_time)?;
        }
        Ok(())
    }
}

impl WellenContainer {
    /// Prepares an owned snapshot sufficient to export `variables` (and only the scopes
    /// needed to reach them) to an FST file.
    pub(crate) fn prepare_fst_export(&self, variables: &[VariableRef]) -> Result<FstExportData> {
        let h: &Hierarchy = &self.hierarchy;

        let mut root = FstExportNode::default();
        let mut seen = AHashSet::new();
        for variable in variables {
            let var_ref = self.get_var_ref(variable)?;
            if !seen.insert(var_ref) {
                continue;
            }
            let mut node = &mut root;
            let mut prefix: Vec<String> = Vec::new();
            for scope in &variable.path.strs {
                prefix.push(scope.clone());
                node = node.children.entry(scope.clone()).or_default();
                if node.id.is_none() {
                    node.id = h.lookup_scope(&prefix);
                }
            }
            node.vars.push(var_ref);
        }
        if seen.is_empty() {
            eyre::bail!("No variables selected for FST export");
        }

        let mut signals = AHashMap::with_capacity(seen.len());
        for var_ref in &seen {
            let signal_ref = h[*var_ref].signal_ref();
            if signals.contains_key(&signal_ref) {
                continue;
            }
            let signal = self.signals.get(&signal_ref).cloned().ok_or_else(|| {
                eyre::eyre!(
                    "Signal for variable '{}' is not loaded",
                    h[*var_ref].name(h)
                )
            })?;
            signals.insert(signal_ref, signal);
        }

        let timescale = h
            .timescale()
            .unwrap_or(Timescale::new(1, TimescaleUnit::Unknown));
        let base_exponent = timescale.unit.to_exponent().unwrap_or(0);
        // The declared multiplier (e.g. the "10" in "10 ns") is restricted by the VCD/FST
        // spec to 1, 10 or 100, so its digits can be folded directly into the exponent
        // without ever needing to multiply the time stamps by a remainder.
        let factor_digits = TimeScale {
            unit: TimeUnit::from(timescale.unit),
            multiplier: Some(timescale.factor),
        }
        .multiplier_digits() as u32;
        let time_scale_shift = common_power_of_ten(&self.time_table);
        let time_scale_divisor = 10u128.pow(time_scale_shift);
        let timescale_exponent = base_exponent
            .saturating_add(factor_digits as i8)
            .saturating_add(time_scale_shift as i8);
        let start_time = scale_time(
            self.time_table.first().copied().unwrap_or(0),
            time_scale_divisor,
        );
        let end_time = scale_time(
            self.time_table.last().copied().unwrap_or(0),
            time_scale_divisor,
        );
        Ok(FstExportData {
            hierarchy: Arc::clone(&self.hierarchy),
            time_table: Arc::clone(&self.time_table),
            root,
            signals,
            version: h.version().to_string(),
            date: h.date().to_string(),
            timescale_exponent,
            time_scale_divisor,
            start_time,
            end_time,
        })
    }
}

/// Counts how many trailing (decimal) zeros `n` has, i.e. the largest `k` for which
/// `10^k` divides `n` evenly. `n` must be non-zero: 0 is a multiple of every power of
/// ten, which would loop forever. Callers filter out zero time stamps beforehand.
fn trailing_zeros_base10(mut n: u128) -> u32 {
    debug_assert_ne!(n, 0, "trailing_zeros_base10(0) would loop forever");
    let mut count = 0;
    while n.is_multiple_of(10) {
        n /= 10;
        count += 1;
    }
    count
}

/// Finds the largest power of ten that evenly divides every non-zero time stamp `t` in
/// `times`. This tells us how much coarser than the source's declared timescale the
/// export's timescale exponent can be (i.e. closer to zero) while still representing
/// every required time stamp as an exact integer.
fn common_power_of_ten(times: &[Time]) -> u32 {
    let mut common = u32::MAX;
    for &t in times {
        if t == 0 {
            continue;
        }
        let zeros = trailing_zeros_base10(t as u128);
        common = common.min(zeros);
        if common == 0 {
            return 0;
        }
    }
    if common == u32::MAX { 0 } else { common }
}

/// Rescales a raw time stamp from the source's timescale to the export's timescale,
/// given the precomputed divisor `10.pow(time_scale_shift)` (see [`common_power_of_ten`]).
fn scale_time(t: Time, divisor: u128) -> Time {
    (t as u128 / divisor) as Time
}

/// Recursively declares `node`'s scopes and variables in the FST header.
fn write_fst_export_node<W: std::io::Write + std::io::Seek>(
    h: &Hierarchy,
    out: &mut FstHeaderWriter<W>,
    node: &FstExportNode,
    signal_ids: &mut AHashMap<SignalRef, FstSignalId>,
) -> Result<()> {
    for var_ref in &node.vars {
        write_fst_export_var(h, out, signal_ids, *var_ref)?;
    }
    for (name, child) in &node.children {
        let (component, tpe) = match child.id {
            Some(id) => {
                let scope = &h[id];
                (
                    scope.component(h).unwrap_or("").to_string(),
                    fst_scope_type(scope.scope_type()),
                )
            }
            None => (String::new(), FstScopeType::Module),
        };
        out.scope(name, &component, tpe)?;
        write_fst_export_node(h, out, child, signal_ids)?;
        out.up_scope()?;
    }
    Ok(())
}

/// Declares a single variable in the FST header, reusing an existing signal id if the
/// same signal was already declared under a different variable name (aliasing).
fn write_fst_export_var<W: std::io::Write + std::io::Seek>(
    h: &Hierarchy,
    out: &mut FstHeaderWriter<W>,
    signal_ids: &mut AHashMap<SignalRef, FstSignalId>,
    var_ref: VarRef,
) -> Result<()> {
    let var = &h[var_ref];
    let name = var.name(h);
    let signal_tpe = match var.signal_encoding(h) {
        SignalEncoding::String => {
            warn!(
                "Variable-length string signal '{name}' is not supported by FST export, skipping it"
            );
            return Ok(());
        }
        SignalEncoding::Real => FstSignalType::real(),
        SignalEncoding::BitVector(len) => FstSignalType::bit_vec(len),
    };
    let tpe = fst_var_type(var.var_type());
    let dir = fst_var_direction(var.direction());
    let signal_ref = var.signal_ref();
    let alias = signal_ids.get(&signal_ref).copied();
    let fst_id = out.var(name, signal_tpe, tpe, dir, alias)?;
    signal_ids.entry(signal_ref).or_insert(fst_id);
    Ok(())
}

/// Maps a wellen scope type to its FST equivalent.
fn fst_scope_type(tpe: ScopeType) -> FstScopeType {
    match tpe {
        ScopeType::Module => FstScopeType::Module,
        ScopeType::Task => FstScopeType::Task,
        ScopeType::Function | ScopeType::VhdlFunction => FstScopeType::Function,
        ScopeType::Begin => FstScopeType::Begin,
        ScopeType::Fork => FstScopeType::Fork,
        ScopeType::Generate => FstScopeType::Generate,
        ScopeType::Struct | ScopeType::VhdlRecord => FstScopeType::Struct,
        ScopeType::Union => FstScopeType::Union,
        ScopeType::Class => FstScopeType::Class,
        ScopeType::Interface => FstScopeType::Interface,
        ScopeType::Package | ScopeType::VhdlPackage => FstScopeType::Package,
        ScopeType::Program => FstScopeType::Program,
        ScopeType::VhdlArchitecture => FstScopeType::VhdlArchitecture,
        ScopeType::VhdlProcedure => FstScopeType::VhdlProcedure,
        ScopeType::VhdlProcess => FstScopeType::VhdlProcess,
        ScopeType::VhdlBlock => FstScopeType::VhdlBlock,
        ScopeType::VhdlForGenerate => FstScopeType::VhdlForGenerate,
        ScopeType::VhdlIfGenerate => FstScopeType::VhdlIfGenerate,
        ScopeType::VhdlGenerate | ScopeType::GhwGeneric => FstScopeType::VhdlGenerate,
        ScopeType::VhdlArray | ScopeType::SvArray => FstScopeType::Struct,
        _ => FstScopeType::Module,
    }
}

/// Maps a wellen variable type to its FST equivalent.
fn fst_var_type(tpe: VarType) -> FstVarType {
    match tpe {
        VarType::Event | VarType::EventParameter => FstVarType::Event,
        VarType::Integer => FstVarType::Integer,
        VarType::Parameter => FstVarType::Parameter,
        VarType::Real | VarType::RealParameter => FstVarType::Real,
        VarType::Reg => FstVarType::Reg,
        VarType::Supply0 => FstVarType::Supply0,
        VarType::Supply1 => FstVarType::Supply1,
        VarType::Time => FstVarType::Time,
        VarType::Tri => FstVarType::Tri,
        VarType::TriAnd => FstVarType::TriAnd,
        VarType::TriOr => FstVarType::TriOr,
        VarType::TriReg => FstVarType::TriReg,
        VarType::Tri0 => FstVarType::Tri0,
        VarType::Tri1 => FstVarType::Tri1,
        VarType::WAnd => FstVarType::Wand,
        VarType::Wire => FstVarType::Wire,
        VarType::WOr => FstVarType::Wor,
        VarType::String => FstVarType::GenericString,
        VarType::Port => FstVarType::Port,
        VarType::SparseArray => FstVarType::SparseArray,
        VarType::RealTime => FstVarType::RealTime,
        VarType::Bit | VarType::Boolean | VarType::StdLogic | VarType::StdULogic => FstVarType::Bit,
        VarType::Logic
        | VarType::BitVector
        | VarType::StdLogicVector
        | VarType::StdULogicVector => FstVarType::Logic,
        VarType::Int => FstVarType::Int,
        VarType::ShortInt => FstVarType::ShortInt,
        VarType::LongInt => FstVarType::LongInt,
        VarType::Byte => FstVarType::Byte,
        VarType::Enum => FstVarType::Enum,
        VarType::ShortReal => FstVarType::ShortReal,
    }
}

/// Maps a wellen variable direction to its FST equivalent.
fn fst_var_direction(dir: VarDirection) -> FstVarDirection {
    match dir {
        VarDirection::Unknown | VarDirection::Implicit => FstVarDirection::Implicit,
        VarDirection::Input => FstVarDirection::Input,
        VarDirection::Output => FstVarDirection::Output,
        VarDirection::InOut => FstVarDirection::InOut,
        VarDirection::Buffer => FstVarDirection::Buffer,
        VarDirection::Linkage => FstVarDirection::Linkage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_zeros_base10_counts_correctly() {
        assert_eq!(trailing_zeros_base10(1), 0);
        assert_eq!(trailing_zeros_base10(7), 0);
        assert_eq!(trailing_zeros_base10(10), 1);
        assert_eq!(trailing_zeros_base10(120), 1);
        assert_eq!(trailing_zeros_base10(100), 2);
        assert_eq!(trailing_zeros_base10(123_000), 3);
        assert_eq!(trailing_zeros_base10(1_000_000_000_000), 12);
    }

    #[test]
    fn common_power_of_ten_of_empty_or_all_zero_is_zero() {
        assert_eq!(common_power_of_ten(&[]), 0);
        assert_eq!(common_power_of_ten(&[0, 0, 0]), 0);
    }

    #[test]
    fn common_power_of_ten_ignores_zero_time_stamps() {
        // The zero time stamp shouldn't constrain the result.
        assert_eq!(common_power_of_ten(&[0, 100, 200]), 2);
    }

    #[test]
    fn common_power_of_ten_uses_the_smallest_shared_shift() {
        assert_eq!(common_power_of_ten(&[10, 20, 30]), 1);
        assert_eq!(common_power_of_ten(&[100, 200, 300]), 2);
        // one non-round time stamp caps the shift for all of them
        assert_eq!(common_power_of_ten(&[100, 200, 305]), 0);
    }

    #[test]
    fn common_power_of_ten_short_circuits_at_zero() {
        assert_eq!(common_power_of_ten(&[1, 100, 1_000_000]), 0);
    }

    #[test]
    fn scale_time_divides_by_the_given_divisor() {
        assert_eq!(scale_time(1_234, 1), 1_234);
        assert_eq!(scale_time(1_230, 10), 123);
        assert_eq!(scale_time(1_200_000, 100), 12_000);
        assert_eq!(scale_time(0, 1000), 0);
    }

    #[test]
    fn fst_scope_type_maps_common_variants() {
        assert_eq!(fst_scope_type(ScopeType::Module), FstScopeType::Module);
        assert_eq!(fst_scope_type(ScopeType::Struct), FstScopeType::Struct);
        assert_eq!(fst_scope_type(ScopeType::VhdlRecord), FstScopeType::Struct);
        assert_eq!(fst_scope_type(ScopeType::VhdlArray), FstScopeType::Struct);
        assert_eq!(fst_scope_type(ScopeType::Unknown), FstScopeType::Module);
    }

    #[test]
    fn fst_var_type_maps_common_variants() {
        assert_eq!(fst_var_type(VarType::Wire), FstVarType::Wire);
        assert_eq!(fst_var_type(VarType::Reg), FstVarType::Reg);
        assert_eq!(fst_var_type(VarType::Event), FstVarType::Event);
        assert_eq!(fst_var_type(VarType::EventParameter), FstVarType::Event);
        assert_eq!(fst_var_type(VarType::Bit), FstVarType::Bit);
        assert_eq!(fst_var_type(VarType::Logic), FstVarType::Logic);
        assert_eq!(fst_var_type(VarType::BitVector), FstVarType::Logic);
    }

    #[test]
    fn fst_var_direction_maps_common_variants() {
        assert_eq!(
            fst_var_direction(VarDirection::Unknown),
            FstVarDirection::Implicit
        );
        assert_eq!(
            fst_var_direction(VarDirection::Implicit),
            FstVarDirection::Implicit
        );
        assert_eq!(
            fst_var_direction(VarDirection::Input),
            FstVarDirection::Input
        );
        assert_eq!(
            fst_var_direction(VarDirection::Output),
            FstVarDirection::Output
        );
        assert_eq!(
            fst_var_direction(VarDirection::InOut),
            FstVarDirection::InOut
        );
    }
}
