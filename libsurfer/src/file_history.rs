#[cfg(not(target_arch = "wasm32"))]
use directories::ProjectDirs;
use egui::Ui;
use indexmap::IndexSet;
use log::info;
#[cfg(not(test))]
use log::warn;
use serde::{Deserialize, Serialize};

use crate::{
    message::Message,
    wave_source::{LoadOptions, WaveSource},
};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
#[cfg(not(target_arch = "wasm32"))]
pub struct FileHistory {
    set: IndexSet<WaveSource>,
    capacity: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            set: IndexSet::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, item: WaveSource) {
        if let WaveSource::Data = item {
            return;
        }
        if self.set.contains(&item) {
            self.set.shift_remove(&item);
        }
        self.set.shift_insert(0, item);
        if self.set.len() > self.capacity {
            self.set.shift_remove_index(self.capacity);
        }
        // Do not save during tests.
        #[cfg(not(test))]
        self.save_to_ron();
    }

    #[cfg(test)]
    fn as_vec(&self) -> Vec<&WaveSource> {
        self.set.iter().collect()
    }

    #[cfg(not(test))]
    fn save_to_ron(&self) {
        if let Some(proj_dirs) = ProjectDirs::from("org", "surfer-project", "surfer") {
            let path = proj_dirs.data_local_dir().join("file_history.ron");
            if let Ok(s) = ron::ser::to_string(self) {
                if std::fs::write(path, s).is_err() {
                    let path = proj_dirs.data_local_dir();
                    info!("Creating local data directory {}", path.display());
                    let _ = std::fs::create_dir_all(path);
                    self.save_to_ron();
                }
            } else {
                warn!("Cannot serialize file history.")
            }
        }
    }

    pub fn load_from_ron() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(proj_dirs) = ProjectDirs::from("org", "surfer-project", "surfer") {
                let path = proj_dirs.data_local_dir().join("file_history.ron");
                if let Ok(s) = std::fs::read_to_string(path) {
                    if let Ok(t) = ron::de::from_str(&s) {
                        return t;
                    }
                }
            }
            info!("Cannot read file history");
            FileHistory::new(5)
        }
    }

    pub fn menu(&self, ui: &mut Ui, msgs: &mut Vec<Message>, is_menu: bool) {
        for recent_file in &self.set {
            let message = match recent_file {
                WaveSource::File(filename) => {
                    Some(Message::LoadFile(filename.clone(), LoadOptions::clean()))
                }
                WaveSource::Url(url) => Some(Message::LoadCommandFileFromUrl(url.clone())),
                WaveSource::DragAndDrop(_) => None,
                WaveSource::Cxxrtl(kind) => Some(Message::SetupCxxrtl(kind.clone())),
                WaveSource::Data => None,
            };
            if let Some(message) = message {
                ui.label(recent_file.to_string()).clicked().then(|| {
                    if is_menu {
                        ui.close_menu();
                    }
                    msgs.push(message);
                });
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl IntoIterator for FileHistory {
    type Item = WaveSource;
    type IntoIter = indexmap::set::IntoIter<WaveSource>;

    fn into_iter(self) -> Self::IntoIter {
        self.set.into_iter()
    }
}

/// Dummy version for wasm32
#[cfg(target_arch = "wasm32")]
pub struct FileHistory;
#[cfg(target_arch = "wasm32")]
impl FileHistory {
    pub fn new(_capacity: usize) -> Self {
        FileHistory
    }

    pub fn push(&mut self, _item: WaveSource) {}

    pub fn load_from_ron() -> Self {
        FileHistory::new(5)
    }
}

#[cfg(test)]
mod test {

    use camino::Utf8PathBuf;

    use crate::wave_source::WaveSource;

    use super::FileHistory;

    #[test]
    fn test_file_history() {
        let a = WaveSource::File(Utf8PathBuf::from("a"));
        let b = WaveSource::File(Utf8PathBuf::from("b"));
        let c = WaveSource::Url("c".to_string());
        let d = WaveSource::Url("d".to_string());

        let mut fh = FileHistory::new(3);
        fh.push(a.clone());
        fh.push(b.clone());
        assert_eq!(fh.as_vec(), [&b.clone(), &a.clone()]);

        fh.push(a.clone());
        assert_eq!(fh.as_vec(), [&a.clone(), &b.clone()]);

        fh.push(c.clone());
        fh.push(d.clone());
        assert_eq!(fh.as_vec(), [&d.clone(), &c.clone(), &a.clone()]);
    }
}
