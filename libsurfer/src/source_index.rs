//! Immutable, validated design metadata attached to one waveform document.

use crate::wave_container::{VariableRef, VariableRefExt};
#[cfg(not(target_arch = "wasm32"))]
use camino::Utf8Path;
use camino::Utf8PathBuf;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceLocation {
    pub file: Utf8PathBuf,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceIndex {
    locations: HashMap<String, SourceLocation>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) database: std::sync::Arc<vtr_vdb::Database>,
    #[cfg(not(target_arch = "wasm32"))]
    base: Utf8PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    symbols: HashMap<String, String>,
}

impl SourceIndex {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn discover(trace: &Utf8Path, reader: &vtr::Reader) -> Option<Self> {
        // A named companion always takes precedence. Never silently attach an
        // unrelated design.vdb or fall back after an invalid companion.
        let path = sibling_candidates(trace).find(|path| path.exists())?;
        Self::attach(&path, reader)
            .map_err(|error| tracing::warn!(%error, %path, "Ignoring invalid VDB companion"))
            .ok()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn attach(path: &Utf8Path, reader: &vtr::Reader) -> Result<Self, String> {
        let database = vtr_vdb::Database::open(path).map_err(|error| error.to_string())?;
        let debugger = vtr_vdb::Debugger::attach(&database, reader, "")?;
        for diagnostic in debugger.diagnostics {
            tracing::warn!(%diagnostic, "VDB attachment");
        }
        let base = path.parent().unwrap_or_else(|| Utf8Path::new("."));
        let mut locations = HashMap::new();
        let mut by_signal = HashMap::new();
        let mut symbols = HashMap::new();
        let mut symbols_by_signal = HashMap::new();
        let nodes: Vec<_> = reader
            .hierarchy()
            .ids()
            .filter_map(|node| {
                let vtr::NodeData::Var { signal, .. } = reader.hierarchy().node(node).data else {
                    return None;
                };
                let path = reader.full_path(node, ".");
                Some((path.split(" [").next().unwrap_or(&path).to_owned(), signal))
            })
            .collect();
        let signals: HashMap<_, _> = nodes.iter().cloned().collect();
        for (symbol_path, symbol) in &database.symbols {
            let recorded = match &database.trace_binding {
                Some(binding) => match binding.signals.get(symbol_path) {
                    Some(path) => path,
                    None => continue,
                },
                None => symbol_path,
            };
            let Some(signal) = signals.get(recorded) else {
                continue;
            };
            let file = Utf8PathBuf::from(&symbol.source.file);
            let location = SourceLocation {
                file: if file.is_absolute() {
                    file
                } else {
                    base.join(file)
                },
                line: symbol.source.line,
                column: symbol.source.column,
            };
            locations.insert(recorded.clone(), location.clone());
            symbols.insert(recorded.clone(), symbol_path.clone());
            symbols_by_signal
                .entry(*signal)
                .or_insert_with(|| symbol_path.clone());
            by_signal.entry(*signal).or_insert(location);
        }
        // Aliases share samples but retain their own declaration where available.
        for (path, signal) in nodes {
            if let Some(symbol) = symbols_by_signal.get(&signal) {
                symbols
                    .entry(path.clone())
                    .or_insert_with(|| symbol.clone());
            }
            if let Some(location) = by_signal.get(&signal) {
                locations.entry(path).or_insert_with(|| location.clone());
            }
        }
        Ok(Self {
            locations,
            database: std::sync::Arc::new(database),
            base: base.to_owned(),
            symbols,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn schematic_symbol(&self, variable: &VariableRef) -> Option<(&str, &str)> {
        let symbol = self.symbols.get(&variable.full_path_string_no_index())?;
        Some((&self.database.symbols.get(symbol)?.owner, symbol))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn schematic_scope(&self, recorded: &str) -> Option<&str> {
        let prefix = self
            .database
            .trace_binding
            .as_ref()
            .map_or("", |binding| binding.prefix.trim_matches('.'));
        let path = if prefix.is_empty() {
            recorded
        } else if recorded == prefix {
            self.database.top.as_str()
        } else {
            recorded.strip_prefix(prefix)?.strip_prefix('.')?
        };
        self.database
            .instances
            .iter()
            .filter(|instance| {
                path == instance.path
                    || path
                        .strip_prefix(&instance.path)
                        .is_some_and(|suffix| suffix.starts_with('.'))
            })
            .max_by_key(|instance| instance.path.len())
            .map(|instance| instance.path.as_str())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn recorded_scope(&self, instance: &str) -> String {
        let prefix = self
            .database
            .trace_binding
            .as_ref()
            .map_or("", |binding| binding.prefix.trim_matches('.'));
        if prefix.is_empty() {
            instance.to_owned()
        } else {
            format!("{prefix}.{instance}")
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn design_source(&self, source: &vtr_vdb::Source) -> Option<SourceLocation> {
        if source.file.is_empty() || source.line == 0 {
            return None;
        }
        let file = Utf8PathBuf::from(&source.file);
        Some(SourceLocation {
            file: if file.is_absolute() {
                file
            } else {
                self.base.join(file)
            },
            line: source.line,
            column: source.column,
        })
    }

    pub(crate) fn location(&self, variable: &VariableRef) -> Option<SourceLocation> {
        self.locations
            .get(&variable.full_path_string_no_index())
            .cloned()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sibling_candidates(trace: &Utf8Path) -> impl Iterator<Item = Utf8PathBuf> {
    [
        trace.with_extension("vdb"),
        trace.with_extension("vdb.json"),
    ]
    .into_iter()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn schematic_mapping_uses_binding_and_scope_boundaries() {
        let trace = Utf8Path::new("../examples/verilator/pipeline.vtr");
        let reader = vtr::Reader::open(trace).unwrap();
        let index = SourceIndex::discover(trace, &reader).unwrap();
        for (recorded, owner, symbol) in [
            ("TOP.top.u0.q", "top.u0", "top.u0.q"),
            ("TOP.q", "top", "top.q"),
        ] {
            assert_eq!(
                index.schematic_symbol(&VariableRef::from_hierarchy_string(recorded)),
                Some((owner, symbol))
            );
        }
        for (recorded, expected) in [
            ("TOP", Some("top")),
            ("TOP.top", Some("top")),
            ("TOP.top.u0", Some("top.u0")),
            ("TOP.top.u0.internal", Some("top.u0")),
            ("TOP.top.u01", Some("top")),
            ("TOPICAL.top", None),
            ("top.u0", None),
        ] {
            assert_eq!(index.schematic_scope(recorded), expected);
        }
        assert_eq!(index.recorded_scope("top.u0"), "TOP.top.u0");
    }

    #[test]
    fn native_mapping_and_aliases_resolve_to_packaged_sources() {
        let trace = Utf8Path::new("../examples/verilator/pipeline.vtr");
        let reader = vtr::Reader::open(trace).unwrap();
        let index = SourceIndex::discover(trace, &reader).unwrap();
        for (path, line) in [("TOP.top.q", 9), ("TOP.q", 9), ("TOP.top.u0.q", 2)] {
            let location = index
                .location(&VariableRef::from_hierarchy_string(path))
                .unwrap();
            assert!(location.file.exists(), "{}", location.file);
            assert_eq!(location.line, line);
        }
        assert!(
            index
                .location(&VariableRef::from_hierarchy_string("top.q"))
                .is_none()
        );
        let other = vtr::Reader::open("../examples/verilator/operators.vtr").unwrap();
        assert!(SourceIndex::attach(&trace.with_extension("vdb"), &other).is_err());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod companion_tests {
    use super::*;

    #[test]
    fn exact_companions_are_portable_and_invalid_preferred_file_does_not_fall_back() {
        let root = Utf8Path::new("../examples/verilator");
        let directory = tempfile::tempdir().unwrap();
        let base = Utf8Path::from_path(directory.path()).unwrap();
        for (from, to) in [
            ("pipeline.vtr", "moved.vtr"),
            ("pipeline.vdb", "moved.vdb.json"),
            ("pipeline.sv", "pipeline.sv"),
        ] {
            std::fs::copy(root.join(from), base.join(to)).unwrap();
        }
        let trace = base.join("moved.vtr");
        let reader = vtr::Reader::open(&trace).unwrap();
        let index = SourceIndex::discover(&trace, &reader).unwrap();
        assert_eq!(
            index
                .location(&VariableRef::from_hierarchy_string("TOP.top.q"))
                .unwrap()
                .file,
            base.join("pipeline.sv")
        );
        std::fs::copy(root.join("operators.vdb"), base.join("moved.vdb")).unwrap();
        assert!(SourceIndex::discover(&trace, &reader).is_none());
        std::fs::remove_file(base.join("moved.vdb")).unwrap();
        std::fs::rename(base.join("moved.vdb.json"), base.join("design.vdb.json")).unwrap();
        assert!(SourceIndex::discover(&trace, &reader).is_none());
    }
}
