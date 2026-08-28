use camino::Utf8PathBuf;
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
use serde::{Deserialize, Serialize};

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
const FILE_HISTORY_FILE: &str = "file_history.ron";

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredFileHistory {
    files: Vec<String>,
}

#[derive(Debug, Default)]
pub struct FileHistory {
    files: Vec<Utf8PathBuf>,
    max_entries: usize,
}

impl FileHistory {
    #[must_use]
    pub fn load(max_entries: usize) -> Self {
        let mut history = Self {
            files: Vec::new(),
            max_entries,
        };
        history.load_from_disk();
        history.truncate_to_limit();
        history
    }

    #[must_use]
    pub fn files(&self) -> &[Utf8PathBuf] {
        &self.files
    }

    #[must_use]
    pub fn display_labels(&self) -> Vec<String> {
        disambiguated_labels(&self.files)
    }

    pub fn add(&mut self, file: &Utf8PathBuf) {
        if self.max_entries == 0 || is_connection_entry(file) {
            return;
        }

        self.files.retain(|path| path != file);
        self.files.insert(0, file.clone());
        self.truncate_to_limit();
        self.save_to_disk();
    }

    fn truncate_to_limit(&mut self) {
        self.files.truncate(self.max_entries);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    fn load_from_disk(&mut self) {
        let Some(path) = storage_path() else {
            return;
        };

        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };

        let Ok(stored) = ron::from_str::<StoredFileHistory>(&content) else {
            return;
        };

        self.files = stored
            .files
            .into_iter()
            .map(Utf8PathBuf::from)
            .collect::<Vec<_>>();
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn load_from_disk(&mut self) {}

    #[cfg(all(not(target_arch = "wasm32"), not(test)))]
    fn save_to_disk(&self) {
        let Some(path) = storage_path() else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };

        if std::fs::create_dir_all(parent).is_err() {
            return;
        }

        let stored = StoredFileHistory {
            files: self
                .files
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        };
        let Ok(ron) =
            ron::Options::default().to_string_pretty(&stored, ron::ser::PrettyConfig::default())
        else {
            return;
        };

        let _ = std::fs::write(path, ron);
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn save_to_disk(&self) {}
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn storage_path() -> Option<Utf8PathBuf> {
    let dirs = crate::config::PROJECT_DIR.as_ref()?;
    Utf8PathBuf::from_path_buf(dirs.data_local_dir().join(FILE_HISTORY_FILE)).ok()
}

fn is_connection_entry(path: &Utf8PathBuf) -> bool {
    let value = path.as_str();
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("ws://")
        || value.starts_with("wss://")
        || value.starts_with("cxxrtl+tcp://")
}

fn disambiguated_labels(paths: &[Utf8PathBuf]) -> Vec<String> {
    let segments: Vec<Vec<String>> = paths
        .iter()
        .map(|path| {
            path.as_str()
                .split(['/', '\\'])
                .filter(|segment| !segment.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();

    let mut labels = segments
        .iter()
        .map(|parts| {
            parts
                .last()
                .cloned()
                .unwrap_or_else(|| String::from("<unknown>"))
        })
        .collect::<Vec<_>>();

    let mut groups = std::collections::HashMap::<String, Vec<usize>>::new();
    for (idx, parts) in segments.iter().enumerate() {
        if let Some(name) = parts.last() {
            groups.entry(name.clone()).or_default().push(idx);
        }
    }

    for indexes in groups.values() {
        if indexes.len() <= 1 {
            continue;
        }

        let max_depth = indexes
            .iter()
            .map(|idx| segments[*idx].len())
            .max()
            .unwrap_or(1);

        let mut found_unique_depth = None;
        for depth in 2..=max_depth {
            let mut seen = std::collections::HashSet::new();
            let all_unique = indexes.iter().all(|idx| {
                let candidate = label_for_depth(&segments[*idx], depth);
                seen.insert(candidate)
            });
            if all_unique {
                found_unique_depth = Some(depth);
                break;
            }
        }

        if let Some(depth) = found_unique_depth {
            for idx in indexes {
                labels[*idx] = label_for_depth(&segments[*idx], depth);
            }
        } else {
            for idx in indexes {
                labels[*idx] = paths[*idx].to_string();
            }
        }
    }

    labels
}

fn label_for_depth(parts: &[String], depth: usize) -> String {
    let start = parts.len().saturating_sub(depth);
    parts[start..].join("/")
}

#[cfg(test)]
mod tests {
    use super::FileHistory;
    use super::disambiguated_labels;
    use camino::Utf8PathBuf;

    #[test]
    fn keeps_most_recent_on_top() {
        let mut history = FileHistory::load(3);

        history.add(&Utf8PathBuf::from("a.vcd"));
        history.add(&Utf8PathBuf::from("b.vcd"));
        history.add(&Utf8PathBuf::from("a.vcd"));

        assert_eq!(
            history.files(),
            [Utf8PathBuf::from("a.vcd"), Utf8PathBuf::from("b.vcd")]
        );
    }

    #[test]
    fn respects_max_entries() {
        let mut history = FileHistory::load(2);

        history.add(&Utf8PathBuf::from("a.vcd"));
        history.add(&Utf8PathBuf::from("b.vcd"));
        history.add(&Utf8PathBuf::from("c.vcd"));

        assert_eq!(
            history.files(),
            [Utf8PathBuf::from("c.vcd"), Utf8PathBuf::from("b.vcd")]
        );
    }

    #[test]
    fn ignores_connection_entries() {
        let mut history = FileHistory::load(5);

        history.add(&Utf8PathBuf::from("https://surver.example/status"));
        history.add(&Utf8PathBuf::from("ws://surver.example/socket"));
        history.add(&Utf8PathBuf::from("wave.vcd"));

        assert_eq!(history.files(), [Utf8PathBuf::from("wave.vcd")]);
    }

    #[test]
    fn disambiguates_duplicate_file_names() {
        let paths = vec![
            Utf8PathBuf::from("C:/work/a/top.vcd"),
            Utf8PathBuf::from("C:/work/b/top.vcd"),
            Utf8PathBuf::from("C:/work/c/other.vcd"),
        ];

        let labels = disambiguated_labels(&paths);

        assert_eq!(labels[0], "a/top.vcd");
        assert_eq!(labels[1], "b/top.vcd");
        assert_eq!(labels[2], "other.vcd");
    }

    #[test]
    fn disambiguates_duplicate_file_names_linux() {
        let paths = vec![
            Utf8PathBuf::from("/home/user/a/top.vcd"),
            Utf8PathBuf::from("/home/user/b/top.vcd"),
            Utf8PathBuf::from("/home/user/c/other.vcd"),
        ];

        let labels = disambiguated_labels(&paths);

        assert_eq!(labels[0], "a/top.vcd");
        assert_eq!(labels[1], "b/top.vcd");
        assert_eq!(labels[2], "other.vcd");
    }
}
