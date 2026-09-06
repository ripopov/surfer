use super::{
    SchematicTile,
    scene::{Scene, signal},
};
use crate::{
    Message,
    source_index::{SourceIndex, SourceLocation},
};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use std::sync::{Arc, Weak, mpsc};

#[derive(Default)]
pub(super) struct Runtime {
    design: Weak<vtr_vdb::Database>,
    instance: Option<String>,
    pending: Option<mpsc::Receiver<Result<Arc<Scene>, String>>>,
    scene: Option<Arc<Scene>>,
    error: Option<String>,
    camera: Option<Camera>,
    area: Option<Rect>,
    selected: Option<Selection>,
}
#[cfg(test)]
impl Runtime {
    pub(super) fn ready(&self) -> bool {
        self.scene.is_some() && self.pending.is_none()
    }
    pub(super) fn wire_point(&self, symbol: &str) -> Pos2 {
        let scene = self.scene.as_ref().unwrap();
        let segment = scene
            .wires
            .iter()
            .filter(|wire| wire.symbol.as_deref() == Some(symbol))
            .flat_map(|wire| wire.points.windows(2))
            .max_by(|a, b| a[0].distance_sq(a[1]).total_cmp(&b[0].distance_sq(b[1])))
            .unwrap();
        self.camera
            .unwrap()
            .screen(segment[0].lerp(segment[1], 0.5), self.area.unwrap())
    }
    pub(super) fn camera_state(&self) -> (f32, Vec2) {
        let camera = self.camera.unwrap();
        (camera.zoom, camera.pan)
    }
}
#[derive(Clone, PartialEq)]
enum Selection {
    Block(usize),
    Net(String),
    Wire(usize),
}
#[derive(Clone, Copy)]
struct Camera {
    zoom: f32,
    pan: Vec2,
}
impl Camera {
    fn fit(scene: &Scene, area: Rect) -> Self {
        let available = (area.size() - vec2(64.0, 100.0)).max(Vec2::splat(1.0));
        let zoom = (available.x / scene.bounds.width())
            .min(available.y / scene.bounds.height())
            .clamp(0.08, 1.4);
        Self {
            zoom,
            pan: area.size() * 0.5 + vec2(0.0, 12.0) - scene.bounds.center().to_vec2() * zoom,
        }
    }
    fn initial(scene: &Scene, area: Rect, symbol: Option<&str>) -> Self {
        let mut camera = Self::fit(scene, area);
        if camera.zoom >= 0.65 {
            return camera;
        }
        if let Some(symbol) = symbol
            && let Some((id, node)) = scene
                .nodes
                .iter()
                .enumerate()
                .find(|(i, _)| signal(&scene.netlist.blocks[*i]) == Some(symbol))
        {
            let mut bounds = node.rect;
            for wire in &scene.netlist.wires {
                if wire.source.0 == id {
                    bounds = bounds.union(scene.nodes[wire.target.0].rect);
                }
                if wire.target.0 == id {
                    bounds = bounds.union(scene.nodes[wire.source.0].rect);
                }
            }
            camera.zoom = ((area.width() - 80.0) / bounds.width())
                .min((area.height() - 120.0) / bounds.height())
                .clamp(0.65, 1.25);
            camera.pan = area.size() * 0.5 - bounds.center().to_vec2() * camera.zoom;
        }
        camera
    }
    fn screen(self, point: Pos2, area: Rect) -> Pos2 {
        area.min + self.pan + point.to_vec2() * self.zoom
    }
    fn world(self, point: Pos2, area: Rect) -> Pos2 {
        ((point - area.min - self.pan) / self.zoom).to_pos2()
    }
    fn rect(self, rect: Rect, area: Rect) -> Rect {
        Rect::from_min_max(self.screen(rect.min, area), self.screen(rect.max, area))
    }
    fn zoom_at(&mut self, anchor: Pos2, area: Rect, factor: f32) {
        let world = self.world(anchor, area);
        self.zoom = (self.zoom * factor).clamp(0.04, 5.0);
        self.pan = anchor - area.min - world.to_vec2() * self.zoom;
    }
}

pub(super) fn draw(
    tile: &SchematicTile,
    ui: &mut Ui,
    index: Option<&SourceIndex>,
    msgs: &mut Vec<Message>,
) {
    let Some(index) = index else {
        *tile.runtime.borrow_mut() = Runtime::default();
        ui.centered_and_justified(|ui| {
            ui.label("Open a VTR recording with a matching VDB to explore its schematic.");
        });
        return;
    };
    let Some(instance) = &tile.instance else {
        ui.centered_and_justified(|ui| {
            ui.label("Right-click a signal or a hierarchy scope and choose Open schematic.");
        });
        return;
    };
    let mut runtime = tile.runtime.borrow_mut();
    let design = Arc::downgrade(&index.database);
    if !Weak::ptr_eq(&runtime.design, &design) || runtime.instance.as_ref() != Some(instance) {
        *runtime = Runtime {
            design,
            instance: Some(instance.clone()),
            selected: tile.highlight.clone().map(Selection::Net),
            ..Default::default()
        };
        let (sender, receiver) = mpsc::channel();
        runtime.pending = Some(receiver);
        let database = index.database.clone();
        let instance = instance.clone();
        let context = ui.ctx().clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Scene::build(&database, &instance)
            }))
            .unwrap_or_else(|_| Err("The layout engine could not arrange this module.".into()))
            .map(Arc::new);
            let _ = sender.send(result);
            context.request_repaint();
        });
    }
    if let Some(result) = runtime
        .pending
        .as_ref()
        .and_then(|receiver| receiver.try_recv().ok())
    {
        runtime.pending = None;
        match result {
            Ok(scene) => runtime.scene = Some(scene),
            Err(error) => runtime.error = Some(error),
        }
    }
    let (area, response) = ui.allocate_exact_size(
        ui.available_size().max(Vec2::splat(1.0)),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(area);
    let dark = ui.visuals().dark_mode;
    let background = if dark {
        Color32::from_rgb(15, 23, 34)
    } else {
        Color32::from_rgb(246, 248, 251)
    };
    let ink = if dark {
        Color32::from_rgb(222, 233, 243)
    } else {
        Color32::from_rgb(26, 46, 66)
    };
    let muted = if dark {
        Color32::from_rgb(124, 146, 169)
    } else {
        Color32::from_rgb(92, 113, 135)
    };
    let accent = Color32::from_rgb(246, 191, 87);
    painter.rect_filled(area, 0.0, background);
    let Some(scene) = runtime.scene.clone() else {
        let text = runtime.error.as_deref().unwrap_or("Arranging schematic…");
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(15.0),
            ink,
        );
        return;
    };
    let mut camera = runtime
        .camera
        .unwrap_or_else(|| Camera::initial(&scene, area, tile.highlight.as_deref()));
    if let Some(previous) = runtime.area {
        camera.pan += (area.size() - previous.size()) * 0.5;
    }
    runtime.area = Some(area);
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    if response.hovered()
        && let Some(pointer) = response.hover_pos()
    {
        let factor = ui.input(|input| {
            let pinch = input.zoom_delta();
            if pinch != 1.0 {
                pinch
            } else {
                (input.smooth_scroll_delta.y * 0.003).exp()
            }
        });
        if factor != 1.0 {
            camera.zoom_at(pointer, area, factor);
        }
    }
    if response.dragged_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Middle)
    {
        camera.pan += ui.input(|i| i.pointer.delta());
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let hovered = response
        .hover_pos()
        .and_then(|p| hit(&scene, camera.world(p, area), 7.0 / camera.zoom));
    if response.clicked() || response.secondary_clicked() {
        runtime.selected = hovered.clone();
    }
    if response.double_clicked() {
        if let Some(Selection::Block(id)) = &hovered {
            if let Some(child) = &scene.netlist.blocks[*id].child {
                msgs.push(Message::OpenSchematic(child.clone(), None));
            }
        } else if hovered.is_none() {
            camera = Camera::fit(&scene, area);
        }
    }
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::F)) {
        camera = Camera::fit(&scene, area);
    }
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let grid = 28.0 * camera.zoom;
    if grid >= 12.0 {
        let origin = area.min + camera.pan;
        let color = if dark {
            Color32::from_rgb(35, 47, 62)
        } else {
            Color32::from_rgb(212, 221, 232)
        };
        let mut x = area.left() + (origin.x - area.left()).rem_euclid(grid);
        while x < area.right() {
            let mut y = area.top() + (origin.y - area.top()).rem_euclid(grid);
            while y < area.bottom() {
                painter.circle_filled(pos2(x, y), 0.7, color);
                y += grid;
            }
            x += grid;
        }
    }
    for (i, wire) in scene.wires.iter().enumerate() {
        let active = matches!(&runtime.selected, Some(Selection::Net(symbol)) if wire.symbol.as_ref() == Some(symbol))
            || runtime.selected == Some(Selection::Wire(i));
        let hover = hovered == Some(Selection::Wire(i))
            || matches!(&hovered, Some(Selection::Net(symbol)) if wire.symbol.as_ref() == Some(symbol));
        let points: Vec<_> = wire
            .points
            .iter()
            .map(|p| camera.screen(*p, area))
            .collect();
        let width = wire
            .symbol
            .as_ref()
            .and_then(|symbol| index.database.symbols.get(symbol))
            .map_or(1, |s| s.ty.width);
        if active {
            painter.add(egui::Shape::line(
                points.clone(),
                Stroke::new(7.0, accent.gamma_multiply(0.16)),
            ));
        }
        painter.add(egui::Shape::line(
            points,
            Stroke::new(
                if active {
                    2.7
                } else if width > 1 {
                    1.8
                } else {
                    1.25
                },
                if active {
                    accent
                } else if hover {
                    ink
                } else {
                    Color32::from_rgb(88, 145, 174)
                },
            ),
        ));
    }
    for (i, node) in scene.nodes.iter().enumerate() {
        let block = &scene.netlist.blocks[i];
        let rect = camera.rect(node.rect, area);
        if !area.intersects(rect.expand(6.0)) {
            continue;
        }
        let is_signal = signal(block).is_some();
        let selected = runtime.selected == Some(Selection::Block(i))
            || matches!(&runtime.selected, Some(Selection::Net(symbol)) if signal(block) == Some(symbol.as_str()));
        let hover = hovered == Some(Selection::Block(i));
        let border = if selected {
            accent
        } else if hover {
            Color32::from_rgb(129, 183, 215)
        } else {
            Color32::from_rgb(64, 85, 111)
        };
        let fill = if dark {
            if block.child.is_some() {
                Color32::from_rgb(31, 48, 68)
            } else {
                Color32::from_rgb(24, 36, 51)
            }
        } else {
            Color32::WHITE
        };
        let radius = if is_signal { 14 } else { 7 };
        painter.rect_filled(
            rect.translate(vec2(0.0, 3.0)),
            radius,
            Color32::BLACK.gamma_multiply(if dark { 0.22 } else { 0.07 }),
        );
        painter.rect(
            rect,
            radius,
            fill,
            Stroke::new(if selected { 2.0 } else { 1.0 }, border),
            StrokeKind::Inside,
        );
        if camera.zoom >= 0.26 {
            let size = (15.0 * camera.zoom).clamp(7.0, 32.0);
            if is_signal {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    short(&block.title, 17),
                    FontId::monospace(size),
                    if selected { accent } else { ink },
                );
            } else {
                let category = if block.child.is_some() {
                    "MODULE"
                } else if block.title.contains("process") {
                    "PROCESS"
                } else {
                    "LOGIC"
                };
                painter.text(
                    rect.min + vec2(12.0, 10.0) * camera.zoom,
                    Align2::LEFT_TOP,
                    category,
                    FontId::proportional((9.0 * camera.zoom).max(6.0)),
                    muted,
                );
                painter.text(
                    rect.min + vec2(12.0, 24.0) * camera.zoom,
                    Align2::LEFT_TOP,
                    short(&block.title, if block.child.is_some() { 26 } else { 20 }),
                    FontId::monospace(size),
                    ink,
                );
                painter.line_segment(
                    [
                        camera.screen(node.rect.min + vec2(0.0, 43.0), area),
                        camera.screen(node.rect.min + vec2(node.rect.width(), 43.0), area),
                    ],
                    Stroke::new(0.7, border.gamma_multiply(0.55)),
                );
            }
            for (pin, position) in block.pins.iter().zip(&node.pins) {
                let center = camera.screen(*position, area);
                painter.circle_filled(
                    center,
                    (2.6 * camera.zoom).clamp(1.4, 5.0),
                    if selected {
                        accent
                    } else {
                        Color32::from_rgb(111, 179, 193)
                    },
                );
                if !is_signal && camera.zoom >= 0.45 {
                    painter.text(
                        center + vec2(if pin.output { -9.0 } else { 9.0 }, 0.0) * camera.zoom,
                        if pin.output {
                            Align2::RIGHT_CENTER
                        } else {
                            Align2::LEFT_CENTER
                        },
                        short(&pin.name, 18),
                        FontId::monospace((12.0 * camera.zoom).max(7.0)),
                        muted,
                    );
                }
            }
        }
    }
    let header = Rect::from_min_size(
        area.min + vec2(12.0, 10.0),
        vec2((area.width() - 24.0).max(1.0), 30.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
        ui.horizontal(|ui| {
            if let Some(parent) = index
                .database
                .instances
                .iter()
                .find(|i| i.path == *instance)
                .and_then(|i| i.parent.as_ref())
                && ui.button("Up").on_hover_text("Parent module").clicked()
            {
                msgs.push(Message::OpenSchematic(parent.clone(), None));
            }
            egui::Frame::new()
                .fill(background.gamma_multiply(0.94))
                .corner_radius(6)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(instance).monospace().color(ink));
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Fit")
                    .on_hover_text("Fit schematic · F or double-click empty canvas")
                    .clicked()
                {
                    camera = Camera::fit(&scene, area);
                }
            });
        });
    });
    painter.text(
        area.left_bottom() + vec2(14.0, -12.0),
        Align2::LEFT_BOTTOM,
        "Scroll to zoom  ·  Drag to pan  ·  Double-click a module to enter",
        FontId::proportional(10.0),
        muted,
    );
    let selected = runtime.selected.clone();
    response.context_menu(|ui| {
        if let Some(selection) = &selected {
            let source = source_for(selection, &scene, index);
            if ui
                .add_enabled(source.is_some(), egui::Button::new("Go to source"))
                .clicked()
            {
                if let Some(source) = source {
                    let owner = match selection {
                        Selection::Block(id) => scene.netlist.blocks[*id]
                            .child
                            .as_deref()
                            .unwrap_or(instance),
                        Selection::Net(symbol) => index
                            .database
                            .symbols
                            .get(symbol)
                            .map_or(instance.as_str(), |s| &s.owner),
                        Selection::Wire(_) => instance,
                    };
                    msgs.push(Message::OpenSource {
                        file: source.file,
                        line: source.line,
                        column: source.column,
                        instance: Some(owner.to_owned()),
                    });
                }
                ui.close();
            }
            if ui.button("Reveal in hierarchy").clicked() {
                let owner = match selection {
                    Selection::Block(id) => scene.netlist.blocks[*id]
                        .child
                        .as_deref()
                        .unwrap_or(instance),
                    Selection::Net(symbol) => index
                        .database
                        .symbols
                        .get(symbol)
                        .map_or(instance.as_str(), |s| &s.owner),
                    Selection::Wire(_) => instance,
                };
                msgs.push(Message::RevealSchematicHierarchy(
                    index.recorded_scope(owner),
                ));
                ui.close();
            }
        } else if ui.button("Fit schematic").clicked() {
            camera = Camera::fit(&scene, area);
            ui.close();
        }
    });
    if let Some(Selection::Block(id)) = hovered {
        response.on_hover_text(format!(
            "{}\n{}",
            scene.netlist.blocks[id].title, scene.netlist.blocks[id].detail
        ));
    }
    runtime.camera = Some(camera);
}
fn short(text: &str, count: usize) -> String {
    if text.chars().count() <= count {
        text.into()
    } else {
        format!("{}…", text.chars().take(count - 1).collect::<String>())
    }
}
fn source_for(selection: &Selection, scene: &Scene, index: &SourceIndex) -> Option<SourceLocation> {
    let source = match selection {
        Selection::Block(id) => scene.netlist.blocks[*id].source.as_ref(),
        Selection::Net(symbol) => index.database.symbols.get(symbol).map(|s| &s.source),
        Selection::Wire(id) => scene.wires[*id]
            .symbol
            .as_ref()
            .and_then(|symbol| index.database.symbols.get(symbol))
            .map(|s| &s.source)
            .or_else(|| {
                scene.netlist.blocks[scene.wires[*id].source_block]
                    .source
                    .as_ref()
            }),
    }?;
    index.design_source(source)
}
fn hit(scene: &Scene, point: Pos2, tolerance: f32) -> Option<Selection> {
    if let Some((id, _)) = scene
        .nodes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, node)| node.rect.contains(point))
    {
        return Some(
            signal(&scene.netlist.blocks[id])
                .map_or(Selection::Block(id), |symbol| Selection::Net(symbol.into())),
        );
    }
    scene
        .wires
        .iter()
        .enumerate()
        .filter_map(|(id, wire)| {
            let distance = wire
                .points
                .windows(2)
                .map(|segment| distance(point, segment[0], segment[1]))
                .fold(f32::INFINITY, f32::min);
            (distance <= tolerance).then_some((id, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| {
            scene.wires[id]
                .symbol
                .clone()
                .map_or(Selection::Wire(id), Selection::Net)
        })
}
fn distance(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let vector = b - a;
    let t = ((point - a).dot(vector) / vector.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
    point.distance(a + vector * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_anchor_survives_zoom_and_limits() {
        let area = Rect::from_min_size(pos2(300.0, 24.0), vec2(800.0, 600.0));
        let anchor = pos2(730.0, 280.0);
        let mut camera = Camera {
            zoom: 0.8,
            pan: vec2(-93.0, 72.0),
        };
        let world = camera.world(anchor, area);
        for factor in [1.5, 0.1, 0.001, 10000.0] {
            camera.zoom_at(anchor, area, factor);
            assert!(camera.screen(world, area).distance(anchor) < 0.001);
            assert!((0.04..=5.0).contains(&camera.zoom));
        }
    }
}
