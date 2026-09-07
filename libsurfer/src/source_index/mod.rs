//! Immutable, validated design metadata attached to one waveform document.
//!
//! Two views of one VDB companion live here. The attachment maps recorded signals
//! to elaborated symbols and their declarations. The static source index, written
//! by the simulator build, classifies every token of every design file; joining a
//! token's declaration location to the same location in the attachment gives the
//! elaborated symbols it denotes, in whichever instance the file is viewed.

pub(crate) mod tokens;

use crate::wave_container::{VariableRef, VariableRefExt};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
pub(crate) use tokens::{FileTokens, Modifiers, Span, TokenClass};
use vtr_vdb::{InactiveRange, IndexLocation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceLocation {
    pub file: Utf8PathBuf,
    pub line: u32,
    pub column: u32,
}

/// What the design database declares at one source location.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Declared {
    /// Elaborated symbol paths declared here, one per instance of the module.
    pub symbols: Vec<String>,
    /// Elaborated instance paths whose name is declared here.
    pub instances: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceIndex {
    locations: HashMap<String, SourceLocation>,
    pub(crate) database: Arc<vtr_vdb::Database>,
    base: Utf8PathBuf,
    symbols: HashMap<String, String>,
    /// Static index: legend, indexed files by absolute path, declarations by location.
    legend: tokens::Legend,
    files: HashMap<Utf8PathBuf, usize>,
    declared: HashMap<IndexLocation, Declared>,
    tokens: Arc<Mutex<HashMap<usize, Arc<FileTokens>>>>,
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
            let location = SourceLocation {
                file: absolute(base, &symbol.source.file),
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
        let (legend, files, declared) = Self::static_index(&database, base);
        Ok(Self {
            locations,
            database: Arc::new(database),
            base: base.to_owned(),
            symbols,
            legend,
            files,
            declared,
            tokens: Arc::default(),
        })
    }

    /// Resolves the legend, the indexed files and the declaration join of the static
    /// source index; an absent index leaves every file plain.
    fn static_index(
        database: &vtr_vdb::Database,
        base: &Utf8Path,
    ) -> (
        tokens::Legend,
        HashMap<Utf8PathBuf, usize>,
        HashMap<IndexLocation, Declared>,
    ) {
        let Some(index) = &database.source_index else {
            return Default::default();
        };
        let legend = tokens::Legend::new(&index.classes, &index.modifiers);
        let files = index
            .files
            .iter()
            .enumerate()
            .map(|(number, file)| (absolute(base, &file.path), number))
            .collect();
        let mut declared: HashMap<IndexLocation, Declared> = HashMap::new();
        let locate = |source: &vtr_vdb::Source| {
            index.file_index(&source.file).map(|file| IndexLocation {
                file: file as u32,
                line: source.line,
                column: source.column,
            })
        };
        for (path, symbol) in &database.symbols {
            if let Some(at) = locate(&symbol.source) {
                declared.entry(at).or_default().symbols.push(path.clone());
            }
        }
        for instance in &database.instances {
            if let Some(at) = locate(&instance.source) {
                declared
                    .entry(at)
                    .or_default()
                    .instances
                    .push(instance.path.clone());
            }
        }
        (legend, files, declared)
    }

    /// Frontend that produced the static index, when the VDB carries one.
    pub(crate) fn producer(&self) -> Option<&str> {
        self.database
            .source_index
            .as_ref()
            .map(|index| index.producer.as_str())
    }

    /// Number of the indexed file at `file`, an absolute path.
    fn file_number(&self, file: &Utf8Path) -> Option<usize> {
        self.files
            .get(file)
            .or_else(|| self.files.get(&normalize(file)))
            .copied()
    }

    /// Classified tokens of `file`, decoded once per design.
    pub(crate) fn file_tokens(&self, file: &Utf8Path) -> Option<Arc<FileTokens>> {
        let number = self.file_number(file)?;
        let mut cache = self.tokens.lock().unwrap();
        if let Some(tokens) = cache.get(&number) {
            return Some(tokens.clone());
        }
        let indexed = self.database.source_index.as_ref()?.files.get(number)?;
        let tokens = Arc::new(FileTokens::new(indexed, &self.legend));
        cache.insert(number, tokens.clone());
        Some(tokens)
    }

    /// Generate blocks of `file` that `instance` leaves uninstantiated.
    pub(crate) fn inactive_ranges(&self, file: &Utf8Path, instance: &str) -> Vec<InactiveRange> {
        let Some(number) = self.file_number(file) else {
            return Vec::new();
        };
        self.database
            .source_index
            .as_ref()
            .map(|index| {
                index
                    .inactive_ranges(instance)
                    .into_iter()
                    .filter(|range| range.file as usize == number)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Symbols and instances the design declares at an index location.
    pub(crate) fn declared(&self, at: IndexLocation) -> Option<&Declared> {
        self.declared.get(&at)
    }

    /// An index location as a file to open.
    pub(crate) fn location_of(&self, at: IndexLocation) -> Option<SourceLocation> {
        let index = self.database.source_index.as_ref()?;
        let file = index.files.get(at.file as usize)?;
        Some(SourceLocation {
            file: absolute(&self.base, &file.path),
            line: at.line,
            column: at.column,
        })
    }

    /// Declaration of the module, interface or package called `name`.
    pub(crate) fn definition(&self, name: &str) -> Option<SourceLocation> {
        let at = self.database.source_index.as_ref()?.definition(name)?;
        self.location_of(at)
    }

    /// Module name of an elaborated instance.
    pub(crate) fn instance_definition(&self, instance: &str) -> Option<&str> {
        self.database
            .instances
            .iter()
            .find(|i| i.path == instance)
            .map(|i| i.definition.as_str())
    }

    /// Elaborated instances of the module or interface called `name`, in design order.
    pub(crate) fn instances_of_module(&self, name: &str) -> Vec<String> {
        self.database
            .instances
            .iter()
            .filter(|instance| instance.definition == name)
            .map(|instance| instance.path.clone())
            .collect()
    }

    pub(crate) fn schematic_symbol(&self, variable: &VariableRef) -> Option<(&str, &str)> {
        let symbol = self.symbols.get(&variable.full_path_string_no_index())?;
        Some((&self.database.symbols.get(symbol)?.owner, symbol))
    }

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

    pub(crate) fn design_source(&self, source: &vtr_vdb::Source) -> Option<SourceLocation> {
        if source.file.is_empty() || source.line == 0 {
            return None;
        }
        Some(SourceLocation {
            file: absolute(&self.base, &source.file),
            line: source.line,
            column: source.column,
        })
    }

    pub(crate) fn location(&self, variable: &VariableRef) -> Option<SourceLocation> {
        self.locations
            .get(&variable.full_path_string_no_index())
            .cloned()
    }

    /// Recorded waveform paths of one elaborated symbol path. Struct fields and array
    /// elements are recorded under the aggregate's name, so trailing selections are
    /// dropped until a binding matches and any recorded element of that aggregate counts.
    pub(crate) fn recorded_paths(&self, design_path: &str) -> Vec<String> {
        let recorded_name = |symbol: &str| -> Option<String> {
            match &self.database.trace_binding {
                Some(binding) => binding.signals.get(symbol).cloned(),
                None => Some(symbol.to_owned()),
            }
        };
        let mut candidate = design_path.to_owned();
        loop {
            if let Some(recorded) = recorded_name(&candidate) {
                if self.locations.contains_key(&recorded) {
                    return vec![recorded];
                }
                // Unpacked arrays are recorded element-wise as `name[i]`.
                let mut elements: Vec<_> = self
                    .locations
                    .keys()
                    .filter(|path| {
                        path.strip_prefix(recorded.as_str())
                            .is_some_and(|rest| rest.starts_with('['))
                    })
                    .cloned()
                    .collect();
                if !elements.is_empty() {
                    elements.sort();
                    return elements;
                }
            }
            let Some(cut) = candidate.rfind(['.', '[']) else {
                return Vec::new();
            };
            candidate.truncate(cut);
            if candidate.is_empty() {
                return Vec::new();
            }
        }
    }

    /// The innermost design instance whose path prefixes `design_path`.
    pub(crate) fn owner_of(&self, design_path: &str) -> Option<&vtr_vdb::Instance> {
        self.database
            .instances
            .iter()
            .filter(|instance| {
                design_path == instance.path
                    || design_path
                        .strip_prefix(instance.path.as_str())
                        .is_some_and(|rest| rest.starts_with(['.', '[']))
            })
            .max_by_key(|instance| instance.path.len())
    }

    /// Instances of the same module as `instance`, for switching the viewed context.
    pub(crate) fn sibling_instances(&self, instance: &str) -> Vec<String> {
        let Some(definition) = self
            .database
            .instances
            .iter()
            .find(|i| i.path == instance)
            .map(|i| i.definition.as_str())
        else {
            return Vec::new();
        };
        self.database
            .instances
            .iter()
            .filter(|i| i.definition == definition)
            .map(|i| i.path.clone())
            .collect()
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

/// Where a path recorded relative to the companion lives on disk.
fn absolute(base: &Utf8Path, path: &str) -> Utf8PathBuf {
    let path = Utf8PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// Removes `.` and `..` components without touching the filesystem.
fn normalize(path: &Utf8Path) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            camino::Utf8Component::CurDir => {}
            camino::Utf8Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
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
