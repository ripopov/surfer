//! Adapt immutable VTR data to the shared waveform renderer's typed model.
//! No VCD serialization or parsing: aliases, directions and HDL types are
//! declared directly. Design metadata stays separate from the trace hierarchy.

use camino::Utf8Path;
use std::collections::HashMap;
use vtr::{NodeData, NodeId, Reader, SignalId, SignalKind};

pub struct LoadedVtr {
    pub hierarchy: wellen::Hierarchy,
    pub body: NativeSource,
    pub(crate) source_index: Option<crate::source_index::SourceIndex>,
    pub transactions: Option<crate::transaction_container::TransactionContainer>,
}

pub(crate) fn load(path: &Utf8Path) -> Result<LoadedVtr, String> {
    let mut reader = Reader::open(path).map_err(|error| error.to_string())?;
    let mut builder = wellen::HierarchyBuilder::new(
        timescale(reader.meta().timescale)?,
        Some(&reader.meta().writer),
        Some(&reader.meta().date),
    );
    let mut signals = HashMap::new();
    for root in reader.hierarchy().roots() {
        declare(&reader, root, &mut builder, &mut signals)?;
    }
    let hierarchy = builder.finish();
    let source_index = crate::source_index::SourceIndex::discover(path, &reader);
    let mut transactions = crate::vtr_transactions::from_reader(&reader, &[], &[])?;
    reader.clear_cache();
    let reader = std::sync::Arc::new(std::sync::Mutex::new(reader));
    if let Some(transactions) = &mut transactions {
        transactions.native = Some(crate::vtr_transactions::NativeTransactions::new(
            reader.clone(),
        ));
    }
    Ok(LoadedVtr {
        hierarchy,
        body: NativeSource {
            reader,
            signals: signals
                .into_iter()
                .map(|(id, reference)| (reference, id))
                .collect(),
        },
        source_index,
        transactions,
    })
}

/// Mapped recording plus canonical signal identities; no retained histories.
pub struct NativeSource {
    pub(crate) reader: std::sync::Arc<std::sync::Mutex<Reader>>,
    signals: HashMap<wellen::SignalRef, SignalId>,
}

impl NativeSource {
    pub fn time_range(&self) -> Vec<u64> {
        self.reader
            .lock()
            .unwrap()
            .time_range()
            .map_or_else(Vec::new, |(start, end)| vec![start, end])
    }

    pub fn load(
        &mut self,
        refs: &[wellen::SignalRef],
    ) -> Result<Vec<(wellen::SignalRef, vtr::SignalData)>, String> {
        let ids: Vec<_> = refs.iter().map(|r| self.signals[r]).collect();
        let mut reader = self.reader.lock().unwrap();
        let result = reader.load_signals(&ids).map_err(|e| e.to_string());
        // Release transient decoded pieces even when the query fails.
        reader.clear_cache();
        result.map(|data| refs.iter().copied().zip(data).collect())
    }
}

pub(crate) fn convert_value(
    value: vtr::SignalValue<'_>,
) -> surfer_translation_types::VariableValue {
    use surfer_translation_types::VariableValue;
    match value {
        vtr::SignalValue::Real(value) => VariableValue::BigUint(value.to_bits().into()),
        vtr::SignalValue::Bits { .. } => {
            let ascii = value.to_ascii();
            num::BigUint::parse_bytes(ascii.as_bytes(), 2)
                .map(VariableValue::BigUint)
                .unwrap_or(VariableValue::String(ascii))
        }
        vtr::SignalValue::VarLen(value) => {
            VariableValue::String(String::from_utf8_lossy(value).into_owned())
        }
    }
}

/// Conversion is restricted to explicit FST export and format-parity tests.
/// Native viewing keeps histories in their original immutable representation.
pub(crate) fn export_body(
    hierarchy: &wellen::Hierarchy,
    histories: &[(wellen::SignalRef, vtr::SignalData)],
    range: &[u64],
) -> wellen::viewers::BodyResult {
    let mut encoder = wellen::Encoder::new(hierarchy);
    let mut changes = std::collections::BinaryHeap::new();
    for (index, (_, signal)) in histories.iter().enumerate() {
        if let Some(time) = signal.times().first() {
            changes.push(std::cmp::Reverse((*time, index, 0usize)));
        }
    }
    if let Some(start) = range.first() {
        encoder.time_change(*start);
    }
    while let Some(std::cmp::Reverse((time, index, offset))) = changes.pop() {
        let (reference, signal) = &histories[index];
        encoder.time_change(time);
        let ascii = signal.get(offset).to_ascii();
        let token = match signal.kind() {
            SignalKind::Bits { width: 1, .. } => ascii,
            SignalKind::Bits { .. } => format!("b{ascii}"),
            SignalKind::Real => format!("r{ascii}"),
            SignalKind::VarLen => format!("s{ascii}"),
        };
        encoder.vcd_value_change(*reference, token.as_bytes());
        if let Some(next) = signal.times().get(offset + 1) {
            changes.push(std::cmp::Reverse((*next, index, offset + 1)));
        }
    }
    if let Some(end) = range.last() {
        encoder.time_change(*end);
    }
    let (source, time_table) = encoder.finish();
    wellen::viewers::BodyResult { source, time_table }
}

fn declare(
    reader: &Reader,
    node: NodeId,
    builder: &mut wellen::HierarchyBuilder,
    signals: &mut HashMap<SignalId, wellen::SignalRef>,
) -> Result<(), String> {
    match reader.hierarchy().node(node).data {
        NodeData::Scope {
            scope_type,
            component,
        } => {
            builder.push_scope(
                reader.name(node),
                convert_scope(scope_type),
                Some(reader.str(component)),
            );
            for child in reader.hierarchy().children(node) {
                declare(reader, child, builder, signals)?;
            }
            builder.pop_scope();
        }
        NodeData::Var {
            var_type,
            direction,
            signal,
            ..
        } => {
            let kind = reader
                .signal_kind(signal)
                .map_err(|error| error.to_string())?;
            let encoding = match kind {
                SignalKind::Bits { width, .. } => wellen::SignalEncoding::BitVector(width),
                SignalKind::Real => wellen::SignalEncoding::Real,
                SignalKind::VarLen => wellen::SignalEncoding::String,
            };
            let reference = *signals
                .entry(signal)
                .or_insert_with(|| builder.new_signal(encoding));
            let (name, index) = split_range(reader.name(node));
            let enum_type = reader
                .hierarchy()
                .attrs(node)
                .iter()
                .find_map(|(key, value)| {
                    if reader.str(*key) != "enum_table" {
                        return None;
                    }
                    let vtr::Value::U64(id) = value else {
                        return None;
                    };
                    let id = NodeId(u32::try_from(*id).ok()?);
                    let entries = reader.hierarchy().enum_entries(id)?;
                    let entries: Vec<_> = entries
                        .iter()
                        .map(|(literal, value)| {
                            let value = reader.str(*value);
                            let width = encoding.length().unwrap_or(value.len() as u32) as usize;
                            let fill = match value.as_bytes().first() {
                                Some(b'x' | b'z') => value.chars().next().unwrap(),
                                _ => '0',
                            };
                            let value = format!(
                                "{}{value}",
                                fill.to_string().repeat(width.saturating_sub(value.len()))
                            );
                            (value, reader.str(*literal))
                        })
                        .collect();
                    Some(
                        builder.declare_enum(
                            reader.name(id),
                            entries
                                .iter()
                                .map(|(value, literal)| (value.as_str(), *literal)),
                        ),
                    )
                });
            builder.add_var(
                name,
                convert_var(var_type),
                convert_direction(direction),
                None,
                index,
                enum_type,
                reference,
            );
        }
        NodeData::Stream { .. } | NodeData::Generator | NodeData::EnumTable { .. } => {}
    }
    Ok(())
}

fn split_range(name: &str) -> (&str, Option<wellen::VarIndex>) {
    let Some((base, range)) = name.rsplit_once(" [") else {
        return (name, None);
    };
    let Some(range) = range.strip_suffix(']') else {
        return (name, None);
    };
    let (left, right) = range.split_once(':').unwrap_or((range, range));
    match (left.parse(), right.parse()) {
        (Ok(left), Ok(right)) => (base, Some(wellen::VarIndex::new(left, right))),
        _ => (name, None),
    }
}

pub(crate) fn timescale(exponent: i8) -> Result<wellen::Timescale, String> {
    use wellen::TimescaleUnit::*;
    let base = i32::from(exponent).div_euclid(3) * 3;
    let unit = match base {
        -21 => ZeptoSeconds,
        -18 => AttoSeconds,
        -15 => FemtoSeconds,
        -12 => PicoSeconds,
        -9 => NanoSeconds,
        -6 => MicroSeconds,
        -3 => MilliSeconds,
        0 => Seconds,
        _ => return Err(format!("unsupported VTR timescale exponent {exponent}")),
    };
    Ok(wellen::Timescale::new(
        10u32.pow((i32::from(exponent) - base) as u32),
        unit,
    ))
}

fn convert_scope(value: vtr::ScopeType) -> wellen::ScopeType {
    match value {
        vtr::ScopeType::Module => wellen::ScopeType::Module,
        vtr::ScopeType::Task => wellen::ScopeType::Task,
        vtr::ScopeType::Function => wellen::ScopeType::Function,
        vtr::ScopeType::Begin => wellen::ScopeType::Begin,
        vtr::ScopeType::Fork => wellen::ScopeType::Fork,
        vtr::ScopeType::Generate => wellen::ScopeType::Generate,
        vtr::ScopeType::Struct => wellen::ScopeType::Struct,
        vtr::ScopeType::Union => wellen::ScopeType::Union,
        vtr::ScopeType::Class => wellen::ScopeType::Class,
        vtr::ScopeType::Interface => wellen::ScopeType::Interface,
        vtr::ScopeType::Package => wellen::ScopeType::Package,
        vtr::ScopeType::Program => wellen::ScopeType::Program,
        vtr::ScopeType::VhdlArchitecture => wellen::ScopeType::VhdlArchitecture,
        vtr::ScopeType::VhdlProcedure => wellen::ScopeType::VhdlProcedure,
        vtr::ScopeType::VhdlFunction => wellen::ScopeType::VhdlFunction,
        vtr::ScopeType::VhdlRecord => wellen::ScopeType::VhdlRecord,
        vtr::ScopeType::VhdlProcess => wellen::ScopeType::VhdlProcess,
        vtr::ScopeType::VhdlBlock => wellen::ScopeType::VhdlBlock,
        vtr::ScopeType::VhdlForGenerate => wellen::ScopeType::VhdlForGenerate,
        vtr::ScopeType::VhdlIfGenerate => wellen::ScopeType::VhdlIfGenerate,
        vtr::ScopeType::VhdlGenerate => wellen::ScopeType::VhdlGenerate,
        vtr::ScopeType::VhdlPackage => wellen::ScopeType::VhdlPackage,
        vtr::ScopeType::SvArray => wellen::ScopeType::SvArray,
        _ => wellen::ScopeType::Unknown,
    }
}

fn convert_var(value: vtr::VarType) -> wellen::VarType {
    match value {
        vtr::VarType::Event => wellen::VarType::Event,
        vtr::VarType::Integer => wellen::VarType::Integer,
        vtr::VarType::Parameter => wellen::VarType::Parameter,
        vtr::VarType::Real => wellen::VarType::Real,
        vtr::VarType::RealParameter => wellen::VarType::RealParameter,
        vtr::VarType::Reg => wellen::VarType::Reg,
        vtr::VarType::Supply0 => wellen::VarType::Supply0,
        vtr::VarType::Supply1 => wellen::VarType::Supply1,
        vtr::VarType::Time => wellen::VarType::Time,
        vtr::VarType::Tri => wellen::VarType::Tri,
        vtr::VarType::TriAnd => wellen::VarType::TriAnd,
        vtr::VarType::TriOr => wellen::VarType::TriOr,
        vtr::VarType::TriReg => wellen::VarType::TriReg,
        vtr::VarType::Tri0 => wellen::VarType::Tri0,
        vtr::VarType::Tri1 => wellen::VarType::Tri1,
        vtr::VarType::WAnd => wellen::VarType::WAnd,
        vtr::VarType::Wire => wellen::VarType::Wire,
        vtr::VarType::WOr => wellen::VarType::WOr,
        vtr::VarType::Port => wellen::VarType::Port,
        vtr::VarType::SparseArray => wellen::VarType::SparseArray,
        vtr::VarType::RealTime => wellen::VarType::RealTime,
        vtr::VarType::String => wellen::VarType::String,
        vtr::VarType::Bit => wellen::VarType::Bit,
        vtr::VarType::Logic => wellen::VarType::Logic,
        vtr::VarType::Int => wellen::VarType::Int,
        vtr::VarType::ShortInt => wellen::VarType::ShortInt,
        vtr::VarType::LongInt => wellen::VarType::LongInt,
        vtr::VarType::Byte => wellen::VarType::Byte,
        vtr::VarType::Enum => wellen::VarType::Enum,
        vtr::VarType::ShortReal => wellen::VarType::ShortReal,
        _ => wellen::VarType::Wire,
    }
}

fn convert_direction(value: vtr::Direction) -> wellen::VarDirection {
    match value {
        vtr::Direction::Implicit => wellen::VarDirection::Implicit,
        vtr::Direction::Input => wellen::VarDirection::Input,
        vtr::Direction::Output => wellen::VarDirection::Output,
        vtr::Direction::InOut => wellen::VarDirection::InOut,
        vtr::Direction::Buffer => wellen::VarDirection::Buffer,
        vtr::Direction::Linkage => wellen::VarDirection::Linkage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn closing_shared_views_evicts_data_and_undo_reloads_it() {
        use crate::{
            Message, SystemState,
            tiles::{
                commands::{SplitMode, WorkspaceCommand},
                layout::Direction,
            },
            transaction_container::TransactionStreamRef,
            wave_container::{VariableRef, VariableRefExt, WaveContainer},
            wave_source::LoadOptions,
        };
        use ftr_parser::types::{GeneratorId, StreamId};
        let mut state = SystemState::new_default_config().unwrap();
        let path =
            camino::Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/combined.vtr");
        state.update(Message::LoadFile(path, LoadOptions::Clear));
        async fn wait(state: &mut SystemState) {
            // Model the visible singleton request published by a rendered canvas.
            let visible = state.user.workspace.layout().visible_tiles();
            for id in visible {
                if let Some(crate::tiles::kind::TileKind::Waveform(tile)) = state
                    .user
                    .workspace
                    .tiles_mut()
                    .get_mut(&id)
                    .map(|entry| &mut entry.kind)
                {
                    tile.view.draw_cache.borrow_mut().payloads = vec![1];
                }
            }
            state.handle_async_messages();
            let start = std::time::Instant::now();
            while !state.waves_fully_loaded() {
                state.handle_async_messages();
                state.handle_batch_commands();
                assert!(start.elapsed().as_secs() < 10);
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }
        wait(&mut state).await;
        state.update(Message::AddVariables(vec![
            VariableRef::from_hierarchy_string("top.count"),
        ]));
        state.update(Message::AddStreamOrGenerator(
            TransactionStreamRef::new_gen(StreamId(2), GeneratorId(3), "issue".into()),
        ));
        wait(&mut state).await;
        let counts = |state: &SystemState| {
            let document = state.user.waves.as_ref().unwrap();
            let WaveContainer::Wellen(waves) = document.inner.as_waves().unwrap() else {
                panic!();
            };
            let transactions = document.inner.as_transactions().unwrap();
            (
                waves.native_signals.len(),
                transactions
                    .inner
                    .tx_generators
                    .values()
                    .map(|g| g.transactions.len())
                    .sum::<usize>(),
            )
        };
        let loaded = counts(&state);
        assert_eq!(loaded.0, 1);
        assert!(loaded.1 > 0);
        let first = state.user.workspace.layout().visible_tiles()[0];
        state.update(Message::Workspace(WorkspaceCommand::SplitTile {
            tile: first,
            dir: Direction::Right,
            mode: SplitMode::Linked,
        }));
        let second = state.user.workspace.layout().focused().unwrap();
        state.update(Message::Workspace(WorkspaceCommand::CloseTile(first)));
        wait(&mut state).await;
        assert_eq!(counts(&state), loaded);
        state.update(Message::Workspace(WorkspaceCommand::CloseTile(second)));
        wait(&mut state).await;
        assert_eq!(counts(&state), (0, 0));
        state.update(Message::Undo(1));
        wait(&mut state).await;
        assert_eq!(counts(&state), loaded);
    }

    #[test]
    fn native_loading_and_eviction_follow_canonical_demand() {
        use crate::wellen::{BodyResult, LoadSignalPayload, LoadSignalsResult, WellenContainer};
        let loaded = load(Utf8Path::new("../examples/verilator/pipeline.vtr")).unwrap();
        let mut container = WellenContainer::new(std::sync::Arc::new(loaded.hierarchy), None, None);
        assert!(
            container
                .add_body(BodyResult::Vtr(loaded.body))
                .unwrap()
                .is_none()
        );
        assert!(container.native_signals.is_empty());
        assert!(container.signals.is_empty());
        let vars = container.variables();
        let first = vars[0].clone();
        let first_id = container.signal_ref(&first).unwrap();
        let second = vars
            .iter()
            .find(|v| container.signal_ref(v).unwrap() != first_id)
            .unwrap()
            .clone();
        let run = |cmd: crate::wellen::LoadSignalsCmd| {
            let (refs, identity, payload) = cmd.destruct();
            let LoadSignalPayload::Vtr(mut source) = payload else {
                panic!("native source expected");
            };
            let data = source.load(&refs).unwrap();
            LoadSignalsResult::native(source, data, identity)
        };
        let cmd = container
            .retain_native_variables(&[first.clone(), first.clone()])
            .unwrap();
        container.on_signals_loaded(run(cmd)).unwrap();
        assert_eq!(container.native_signals.len(), 1);
        let owned = container.signal_accessor(first_id).unwrap();
        let expected: Vec<_> = owned.iter_changes().collect();
        assert!(
            container
                .retain_native_variables(std::slice::from_ref(&first))
                .is_none()
        );
        let cmd = container.retain_native_variables(&[second]).unwrap();
        assert!(container.native_signals.is_empty());
        assert!(container.retain_native_variables(&[]).is_none());
        // A completed background load cannot resurrect a removed signal.
        assert!(container.on_signals_loaded(run(cmd)).unwrap().is_none());
        assert!(container.native_signals.is_empty());
        assert_eq!(owned.iter_changes().collect::<Vec<_>>(), expected);
        let cmd = container.retain_native_variables(&[first]).unwrap();
        container.on_signals_loaded(run(cmd)).unwrap();
        assert_eq!(
            container
                .signal_accessor(first_id)
                .unwrap()
                .iter_changes()
                .collect::<Vec<_>>(),
            expected
        );
        container.retain_native_variables(&[]);
        let cmd = container.retain_native_variables(&vars[..1]).unwrap();
        let (_, identity, payload) = cmd.destruct();
        let LoadSignalPayload::Vtr(source) = payload else {
            panic!();
        };
        assert!(
            container
                .on_signals_loaded(LoadSignalsResult::native(source, vec![], identity))
                .unwrap()
                .is_none()
        );
        assert!(
            container.retain_native_variables(&vars[..1]).is_none(),
            "failed loads must not retry in a tight loop"
        );
        container.retain_native_variables(&[]);
        let retry = container.retain_native_variables(&vars[..1]).unwrap();
        container.on_signals_loaded(run(retry)).unwrap();
        assert!(container.is_signal_loaded(first_id));
    }

    #[test]
    fn verilator_examples_match_fst_metadata_and_every_change() {
        for name in ["pipeline", "operators"] {
            let path = format!("../examples/verilator/{name}");
            let mut vtr = load(Utf8Path::new(&format!("{path}.vtr"))).unwrap();
            let fst = wellen::viewers::read_header_from_file(
                format!("{path}.fst"),
                &surver::WELLEN_SURFER_DEFAULT_OPTIONS,
            )
            .unwrap();
            let fst_body = wellen::viewers::read_body(fst.body, &fst.hierarchy, None).unwrap();
            assert_eq!(vtr.hierarchy.timescale(), fst.hierarchy.timescale());
            let inspect = |h: &wellen::Hierarchy, mut body: wellen::viewers::BodyResult| {
                let refs: Vec<_> = h.signals().collect();
                let signals: HashMap<_, _> = body
                    .source
                    .load_signals(&refs, h, false)
                    .into_iter()
                    .map(|signal| (signal.signal_ref(), signal))
                    .collect();
                let scopes: BTreeMap<_, _> = h
                    .all_scopes()
                    .map(|id| {
                        let scope = &h[id];
                        (
                            scope.full_name(h),
                            (scope.scope_type(), scope.component(h).map(str::to_owned)),
                        )
                    })
                    .collect();
                let variables: BTreeMap<_, _> = h
                    .all_vars()
                    .map(|id| {
                        let var = &h[id];
                        let changes: Vec<_> = signals[&var.signal_ref()]
                            .iter_changes()
                            .map(|(index, value)| {
                                (body.time_table[index as usize], format!("{value:?}"))
                            })
                            .collect();
                        let enums = var.enum_type(h).map(|(_, entries)| {
                            entries
                                .into_iter()
                                .map(|(value, label)| (value.to_owned(), label.to_owned()))
                                .collect::<BTreeMap<_, _>>()
                        });
                        (
                            var.full_name(h),
                            (
                                var.var_type(),
                                var.direction(),
                                var.index(),
                                var.signal_encoding(h),
                                enums,
                                changes,
                            ),
                        )
                    })
                    .collect();
                (
                    scopes,
                    variables,
                    body.time_table.first().copied(),
                    body.time_table.last().copied(),
                )
            };
            let refs: Vec<_> = vtr.hierarchy.signals().collect();
            let data = vtr.body.load(&refs).unwrap();
            let body = export_body(&vtr.hierarchy, &data, &vtr.body.time_range());
            assert_eq!(
                inspect(&vtr.hierarchy, body),
                inspect(&fst.hierarchy, fst_body),
                "{name}"
            );
        }
    }
}
