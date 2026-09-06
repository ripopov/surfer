//! A canvas-first schematic tile. The document owns design data; each tile owns
//! navigation and disposable, asynchronously produced layout geometry.
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
mod canvas;
#[cfg(not(target_arch = "wasm32"))]
mod scene;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchematicTile {
    pub instance: Option<String>,
    pub highlight: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    #[serde(skip)]
    runtime: std::cell::RefCell<canvas::Runtime>,
}
impl Clone for SchematicTile {
    fn clone(&self) -> Self {
        Self {
            instance: self.instance.clone(),
            highlight: self.highlight.clone(),
            ..Default::default()
        }
    }
}
impl SchematicTile {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn layout_ready(&self) -> bool {
        self.runtime.borrow().ready()
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn test_wire_point(&self, symbol: &str) -> egui::Pos2 {
        self.runtime.borrow().wire_point(symbol)
    }
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn test_camera(&self) -> (f32, egui::Vec2) {
        self.runtime.borrow().camera_state()
    }

    pub(crate) fn open(&mut self, instance: String, highlight: Option<String>) {
        self.instance = Some(instance);
        self.highlight = highlight;
        #[cfg(not(target_arch = "wasm32"))]
        {
            *self.runtime.get_mut() = Default::default();
        }
    }
    pub(crate) fn ui(
        &self,
        ui: &mut egui::Ui,
        index: Option<&crate::source_index::SourceIndex>,
        msgs: &mut Vec<crate::Message>,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        canvas::draw(self, ui, index, msgs);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (index, msgs);
            ui.centered_and_justified(|ui| {
                ui.label("VDB schematics are available in native Surfer.");
            });
        }
    }
}
