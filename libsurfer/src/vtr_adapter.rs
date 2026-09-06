//! Adapt immutable VTR data to the shared waveform renderer's typed model.
//! No VCD serialization or parsing: aliases, directions and HDL types are
//! declared directly. Design metadata stays separate from the trace hierarchy.

use camino::Utf8Path;
use std::collections::HashMap;
use vtr::{NodeData, NodeId, Reader, SignalId, SignalKind};

pub struct LoadedVtr {
    pub hierarchy: wellen::Hierarchy,
    pub body: wellen::viewers::BodyResult,
    pub(crate) source_index: Option<crate::source_index::SourceIndex>,
    pub transactions: Option<crate::transaction_container::TransactionContainer>,
}

pub(crate) fn load(path: &Utf8Path) -> Result<LoadedVtr, String> {
    let reader = Reader::open(path).map_err(|error| error.to_string())?;
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
    let mut encoder = wellen::Encoder::new(&hierarchy);
    if let Some((start, end)) = reader.time_range() {
        encoder.time_change(start);
        reader
            .for_each_change(start, end, |time, signal, value| {
                let Some(reference) = signals.get(&signal) else {
                    return;
                };
                encoder.time_change(time);
                let ascii = value.to_ascii();
                let value = match reader.signal_kind(signal).expect("declared signal") {
                    SignalKind::Bits { width: 1, .. } => ascii,
                    SignalKind::Bits { .. } => format!("b{ascii}"),
                    SignalKind::Real => format!("r{ascii}"),
                    SignalKind::VarLen => format!("s{ascii}"),
                };
                encoder.vcd_value_change(*reference, value.as_bytes());
            })
            .map_err(|error| error.to_string())?;
        // A quiet tail is still part of the recorded time range.
        encoder.time_change(end);
    }
    let (source, time_table) = encoder.finish();
    Ok(LoadedVtr {
        hierarchy,
        body: wellen::viewers::BodyResult { source, time_table },
        source_index: crate::source_index::SourceIndex::discover(path, &reader),
        transactions: crate::vtr_transactions::from_reader(&reader)?,
    })
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

fn timescale(exponent: i8) -> Result<wellen::Timescale, String> {
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

    #[test]
    fn verilator_examples_match_fst_metadata_and_every_change() {
        for name in ["pipeline", "operators"] {
            let path = format!("../examples/verilator/{name}");
            let vtr = load(Utf8Path::new(&format!("{path}.vtr"))).unwrap();
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
            assert_eq!(
                inspect(&vtr.hierarchy, vtr.body),
                inspect(&fst.hierarchy, fst_body),
                "{name}"
            );
        }
    }
}
