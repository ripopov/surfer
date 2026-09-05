//! Disposable egui_tiles adapter. A UI pass proposes changes; only the workspace
//! dispatcher may commit them to `Layout`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use egui::{Rect, Response, Ui};
use egui_tiles::{
    Behavior, Container, Linear, LinearDir, SimplificationOptions, Tabs, Tile, Tiles, Tree,
    UiResponse,
};

use super::{
    TileId,
    layout::{Layout, LayoutError, LayoutNode, MAX_LAYOUT_DEPTH, MAX_LAYOUT_NODES, SplitDir},
    runtime::WorkspaceRuntime,
};

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error("runtime layout contains a grid")]
    Grid,
    #[error("runtime layout contains a cycle, duplicate node or missing node")]
    InvalidTree,
}

/// Kept separate so the dispatcher can record structural history without
/// treating tab selection or a split resize as an undoable operation.
#[derive(Debug)]
pub struct LayoutEdit {
    pub revision: u64,
    pub root: Option<LayoutNode>,
    pub focused: Option<TileId>,
    pub structural: bool,
    pub moved_tile: Option<TileId>,
}

#[derive(Debug)]
pub enum PaneEvent<M> {
    Focus(TileId),
    Close(TileId),
    Command(M),
}

pub struct LayoutPass<M> {
    pub edit: Option<LayoutEdit>,
    pub events: Vec<PaneEvent<M>>,
    pub rects: BTreeMap<TileId, Rect>,
    pub tab_rects: BTreeMap<TileId, Rect>,
    pub revision: u64,
}

/// The registry supplies this read-only view over entries and shared services.
/// The layout adapter never branches on a concrete tile kind.
pub trait PaneRenderer {
    type Command;
    fn focus_stroke(&self, visuals: &egui::Visuals) -> egui::Stroke {
        visuals.selection.stroke
    }
    fn title(&self, tile: TileId) -> String;
    fn ui(&self, tile: TileId, focused: bool, ui: &mut Ui, commands: &mut Vec<Self::Command>);
    fn tab_context_menu(&self, _tile: TileId, _ui: &mut Ui, _commands: &mut Vec<Self::Command>) {}
    /// Contents of the tab bar's `+` menu; `anchor` is the group's active tile.
    fn tab_bar_menu(&self, _anchor: TileId, _ui: &mut Ui, _commands: &mut Vec<Self::Command>) {}
}

pub struct LayoutAdapter {
    tree: Tree<TileId>,
    revision: Option<u64>,
    drag_origin: Option<Option<LayoutNode>>,
    dragged_tile: Option<TileId>,
}

impl LayoutAdapter {
    pub fn new(id: egui::Id) -> Self {
        Self {
            tree: Tree::empty(id),
            revision: None,
            drag_origin: None,
            dragged_tile: None,
        }
    }

    /// Reconcile only when authority changes. Surviving panes and containers
    /// retain runtime IDs; per-tile widget IDs are salted independently.
    fn reconcile(&mut self, layout: &Layout) {
        if self.revision == Some(layout.revision()) {
            return;
        }
        let mut used = HashSet::new();
        self.tree.root = layout
            .root()
            .map(|root| reconcile_node(root, &mut self.tree.tiles, &mut used));
        let removed = self
            .tree
            .tiles
            .tile_ids()
            .filter(|id| !used.contains(id))
            .collect::<Vec<_>>();
        for id in removed {
            self.tree.tiles.remove(id);
        }
        self.revision = Some(layout.revision());
        self.drag_origin = None;
        self.dragged_tile = None;
    }

    pub fn draw<R: PaneRenderer>(
        &mut self,
        ui: &mut Ui,
        layout: &Layout,
        runtime: &WorkspaceRuntime,
        renderer: &R,
        hide_single_tab_bar: bool,
    ) -> Result<LayoutPass<R::Command>, AdapterError> {
        self.reconcile(layout);
        let cancelled =
            ui.input(|i| i.key_pressed(egui::Key::Escape)) && self.drag_origin.is_some();
        if cancelled {
            ui.ctx().stop_dragging();
            self.revision = None;
            self.reconcile(layout);
        }
        let before = read_tree(&self.tree)?;
        if self.drag_origin.is_none() && self.tree.dragged_id(ui.ctx()).is_some() {
            self.drag_origin = Some(before.clone());
        }
        self.dragged_tile = self
            .tree
            .dragged_id(ui.ctx())
            .and_then(|id| self.tree.tiles.get_pane(&id).copied())
            .or(self.dragged_tile);
        let mut behavior = PaneBehavior {
            renderer,
            runtime,
            focused: layout.focused(),
            events: vec![],
            rects: BTreeMap::new(),
            tab_rects: BTreeMap::new(),
            hide_tab_bar: hide_single_tab_bar && layout.tile_order().len() == 1,
            drag_started: false,
            dragged_tile: None,
            dropped: false,
        };
        self.tree.ui(&mut behavior, ui);
        self.dragged_tile = behavior
            .dragged_tile
            .or_else(|| {
                self.tree
                    .dragged_id(ui.ctx())
                    .and_then(|id| self.tree.tiles.get_pane(&id).copied())
            })
            .or(self.dragged_tile);
        if behavior.drag_started && self.drag_origin.is_none() {
            self.drag_origin = Some(before);
        }
        // Dropping can create bare panes until the library's next UI pass.
        self.tree.simplify(&simplification());
        let after = read_tree(&self.tree)?;
        let dragging =
            self.tree.dragged_id(ui.ctx()).is_some() && !ui.input(|i| i.pointer.any_released());
        let mut edit = None;
        if !dragging {
            let origin = self.drag_origin.take();
            let moved_tile = self.dragged_tile.take();
            if origin.is_some() && !behavior.dropped {
                // A release without a drop discards hover-driven tab activation.
                self.revision = None;
                self.reconcile(layout);
            } else if after.as_ref() != layout.root() || behavior.focused != layout.focused() {
                let structural = !same_topology(layout.root(), after.as_ref());
                edit = Some(LayoutEdit {
                    revision: layout.revision(),
                    root: after,
                    focused: behavior.focused,
                    structural,
                    moved_tile: structural.then_some(moved_tile).flatten(),
                });
            }
        }
        Ok(LayoutPass {
            edit,
            events: behavior.events,
            rects: behavior.rects,
            tab_rects: behavior.tab_rects,
            revision: layout.revision(),
        })
    }
}

fn simplification() -> SimplificationOptions {
    SimplificationOptions {
        prune_empty_tabs: true,
        prune_empty_containers: true,
        prune_single_child_tabs: false,
        prune_single_child_containers: true,
        all_panes_must_have_tabs: true,
        // Surfer owns normalization; retain surviving container identities.
        join_nested_linear_containers: false,
        flatten_tabs_in_tabs: true,
    }
}

struct PaneBehavior<'a, R: PaneRenderer> {
    renderer: &'a R,
    runtime: &'a WorkspaceRuntime,
    focused: Option<TileId>,
    events: Vec<PaneEvent<R::Command>>,
    rects: BTreeMap<TileId, Rect>,
    tab_rects: BTreeMap<TileId, Rect>,
    hide_tab_bar: bool,
    drag_started: bool,
    dragged_tile: Option<TileId>,
    dropped: bool,
}

impl<R: PaneRenderer> PaneBehavior<'_, R> {
    fn focus(&mut self, id: TileId) {
        if self.focused != Some(id) {
            self.focused = Some(id);
            self.events.push(PaneEvent::Focus(id));
        }
    }
}

impl<R: PaneRenderer> Behavior<TileId> for PaneBehavior<'_, R> {
    fn pane_ui(&mut self, ui: &mut Ui, _node: egui_tiles::TileId, pane: &mut TileId) -> UiResponse {
        let rect = ui.max_rect().intersect(ui.clip_rect());
        self.rects.insert(*pane, rect);
        ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
        if ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|pos| rect.contains(pos))
        }) {
            self.focus(*pane);
        }
        let mut commands = Vec::new();
        ui.scope_builder(
            egui::UiBuilder::new()
                .id(self.runtime.egui_id(*pane, "body"))
                .max_rect(rect),
            |ui| {
                ui.set_clip_rect(rect);
                self.renderer
                    .ui(*pane, self.focused == Some(*pane), ui, &mut commands);
            },
        );
        self.events
            .extend(commands.into_iter().map(PaneEvent::Command));
        if self.focused == Some(*pane) && !self.hide_tab_bar {
            ui.painter().rect_stroke(
                rect,
                0.0,
                self.renderer.focus_stroke(ui.visuals()),
                egui::StrokeKind::Inside,
            );
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &TileId) -> egui::WidgetText {
        self.renderer.title(*pane).into()
    }

    fn tab_outline_stroke(
        &self,
        visuals: &egui::Visuals,
        tiles: &Tiles<TileId>,
        id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        if tiles
            .get_pane(&id)
            .is_some_and(|pane| self.focused == Some(*pane))
        {
            self.renderer.focus_stroke(visuals)
        } else if state.active {
            egui::Stroke::new(1.0, visuals.widgets.active.bg_fill)
        } else {
            egui::Stroke::NONE
        }
    }

    fn is_tab_closable(&self, tiles: &Tiles<TileId>, id: egui_tiles::TileId) -> bool {
        tiles.get_pane(&id).is_some()
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<TileId>, id: egui_tiles::TileId) -> bool {
        if let Some(pane) = tiles.get_pane(&id) {
            self.events.push(PaneEvent::Close(*pane));
        }
        false
    }

    fn on_tab_button(
        &mut self,
        tiles: &mut Tiles<TileId>,
        id: egui_tiles::TileId,
        response: Response,
    ) -> Response {
        if self.hide_tab_bar {
            return response;
        }
        if let Some(pane) = tiles.get_pane(&id) {
            self.tab_rects.insert(*pane, response.rect);
            if response.drag_started() {
                self.dragged_tile = Some(*pane);
            }
            if response.clicked() || response.secondary_clicked() || response.drag_started() {
                self.focus(*pane);
            }
            let mut commands = Vec::new();
            response.context_menu(|ui| self.renderer.tab_context_menu(*pane, ui, &mut commands));
            self.events
                .extend(commands.into_iter().map(PaneEvent::Command));
        }
        response
    }

    fn top_bar_right_ui(
        &mut self,
        tiles: &Tiles<TileId>,
        ui: &mut Ui,
        _tile_id: egui_tiles::TileId,
        tabs: &Tabs,
        _scroll_offset: &mut f32,
    ) {
        if self.hide_tab_bar {
            return;
        }
        let Some(anchor) = tabs.active.and_then(|id| tiles.get_pane(&id).copied()) else {
            return;
        };
        let mut commands = Vec::new();
        ui.menu_button("➕", |ui| {
            self.renderer.tab_bar_menu(anchor, ui, &mut commands);
        })
        .response
        .on_hover_text("New tile or split");
        self.events
            .extend(commands.into_iter().map(PaneEvent::Command));
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        if self.hide_tab_bar { 0.0 } else { 24.0 }
    }
    fn simplification_options(&self) -> SimplificationOptions {
        simplification()
    }
    fn is_tile_draggable(&self, tiles: &Tiles<TileId>, id: egui_tiles::TileId) -> bool {
        tiles.get_pane(&id).is_some()
    }
    fn on_edit(&mut self, edit: egui_tiles::EditAction) {
        match edit {
            egui_tiles::EditAction::TileDragged => self.drag_started = true,
            egui_tiles::EditAction::TileDropped => self.dropped = true,
            _ => {}
        }
    }
}

use super::layout::same_topology;

fn reconcile_node(
    node: &LayoutNode,
    tiles: &mut Tiles<TileId>,
    used: &mut HashSet<egui_tiles::TileId>,
) -> egui_tiles::TileId {
    let tile = match node {
        LayoutNode::Tile(pane) => Tile::Pane(*pane),
        LayoutNode::Tabs { active, children } => {
            let children = children
                .iter()
                .map(|child| reconcile_node(child, tiles, used))
                .collect::<Vec<_>>();
            let mut tabs = Tabs::new(children);
            tabs.active = Some(tabs.children[*active]);
            Tile::Container(Container::Tabs(tabs))
        }
        LayoutNode::Split {
            dir,
            shares,
            children,
        } => {
            let children = children
                .iter()
                .map(|child| reconcile_node(child, tiles, used))
                .collect::<Vec<_>>();
            let mut linear = Linear::new(
                match dir {
                    SplitDir::Horizontal => LinearDir::Horizontal,
                    SplitDir::Vertical => LinearDir::Vertical,
                },
                children,
            );
            for (id, share) in linear.children.iter().zip(shares) {
                linear.shares.set_share(*id, *share);
            }
            Tile::Container(Container::Linear(linear))
        }
    };
    // Reuse containers by surviving direct children, preferring the exact set.
    // Pane identity is exact and never depends on its position in the tree.
    let matching = tiles
        .iter()
        .filter(|(id, _)| !used.contains(id))
        .filter_map(|(id, old)| {
            let score = match (&tile, old) {
                (Tile::Pane(a), Tile::Pane(b)) if a == b => usize::MAX,
                (Tile::Container(a), Tile::Container(b)) if a.kind() == b.kind() => {
                    let overlap = a
                        .children()
                        .filter(|child| b.children().any(|other| child == &other))
                        .count();
                    if overlap == 0 {
                        return None;
                    }
                    overlap * 2 + usize::from(a.children().eq(b.children()))
                }
                _ => return None,
            };
            Some((*id, score))
        })
        .max_by_key(|(id, score)| (*score, std::cmp::Reverse(id.0)))
        .map(|(id, _)| id);
    let id = matching.unwrap_or_else(|| tiles.next_free_id());
    tiles.insert(id, tile);
    used.insert(id);
    id
}

fn read_tree(tree: &Tree<TileId>) -> Result<Option<LayoutNode>, AdapterError> {
    fn read(
        tiles: &Tiles<TileId>,
        id: egui_tiles::TileId,
        seen: &mut HashSet<egui_tiles::TileId>,
        depth: usize,
    ) -> Result<LayoutNode, AdapterError> {
        if depth > MAX_LAYOUT_DEPTH || seen.len() >= MAX_LAYOUT_NODES {
            return Err(LayoutError::Limit.into());
        }
        if !seen.insert(id) {
            return Err(AdapterError::InvalidTree);
        }
        Ok(match tiles.get(id).ok_or(AdapterError::InvalidTree)? {
            Tile::Pane(pane) => LayoutNode::Tile(*pane),
            Tile::Container(Container::Grid(_)) => return Err(AdapterError::Grid),
            Tile::Container(Container::Tabs(tabs)) => {
                let active = tabs
                    .active
                    .and_then(|active| tabs.children.iter().position(|id| *id == active))
                    .ok_or(LayoutError::ActiveTab)?;
                LayoutNode::Tabs {
                    active,
                    children: tabs
                        .children
                        .iter()
                        .map(|id| read(tiles, *id, seen, depth + 1))
                        .collect::<Result<_, _>>()?,
                }
            }
            Tile::Container(Container::Linear(linear)) => LayoutNode::Split {
                dir: match linear.dir {
                    LinearDir::Horizontal => SplitDir::Horizontal,
                    LinearDir::Vertical => SplitDir::Vertical,
                },
                shares: linear
                    .children
                    .iter()
                    .map(|id| linear.shares[*id])
                    .collect(),
                children: linear
                    .children
                    .iter()
                    .map(|id| read(tiles, *id, seen, depth + 1))
                    .collect::<Result<_, _>>()?,
            },
        })
    }
    let raw = tree
        .root
        .map(|root| read(&tree.tiles, root, &mut HashSet::new(), 1))
        .transpose()?;
    let ids = tree
        .tiles
        .tiles()
        .filter_map(|tile| match tile {
            Tile::Pane(id) => Some(*id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let normalized = Layout::from_file(
        super::layout::LayoutFile {
            root: raw,
            focused: None,
            focus_history: vec![],
        },
        &ids,
    )?;
    Ok(normalized.to_file().root)
}

#[cfg(test)]
mod tests {
    use super::super::layout::{Direction, Placement};

    #[derive(Default)]
    struct Probe {
        seen: std::cell::RefCell<BTreeMap<TileId, (egui::Id, Rect)>>,
        hide_single_tab_bar: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ProbeCommand {
        Pressed(TileId, bool),
        NewTab(TileId),
    }

    impl PaneRenderer for Probe {
        type Command = ProbeCommand;

        fn title(&self, tile: TileId) -> String {
            format!("Tile {}", tile.0)
        }

        fn ui(&self, tile: TileId, focused: bool, ui: &mut Ui, commands: &mut Vec<Self::Command>) {
            self.seen
                .borrow_mut()
                .insert(tile, (ui.id(), ui.clip_rect()));
            if ui.input(|i| i.pointer.any_pressed()) {
                commands.push(ProbeCommand::Pressed(tile, focused));
            }
            ui.label(format!("Body {}", tile.0));
        }

        fn tab_bar_menu(&self, anchor: TileId, ui: &mut Ui, commands: &mut Vec<Self::Command>) {
            if ui.button("Add tab").clicked() {
                commands.push(ProbeCommand::NewTab(anchor));
                ui.close();
            }
        }
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> Option<egui::Pos2> {
        fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.pos + egui::vec2(4.0, 4.0))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, text)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, text))
    }

    fn frame(
        ctx: &egui::Context,
        adapter: &mut LayoutAdapter,
        layout: &Layout,
        runtime: &WorkspaceRuntime,
        probe: &Probe,
        events: Vec<egui::Event>,
    ) -> LayoutPass<ProbeCommand> {
        frame_with_output(ctx, adapter, layout, runtime, probe, events).0
    }

    fn frame_with_output(
        ctx: &egui::Context,
        adapter: &mut LayoutAdapter,
        layout: &Layout,
        runtime: &WorkspaceRuntime,
        probe: &Probe,
        events: Vec<egui::Event>,
    ) -> (LayoutPass<ProbeCommand>, egui::FullOutput) {
        let mut pass = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    pass = Some(
                        adapter
                            .draw(ui, layout, runtime, probe, probe.hide_single_tab_bar)
                            .unwrap(),
                    );
                });
            },
        );
        output.textures_delta.clear();
        (pass.unwrap(), output)
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn commit(layout: &mut Layout, pass: LayoutPass<ProbeCommand>) {
        if let Some(edit) = pass.edit {
            layout
                .apply_proposal(edit.revision, edit.root, edit.focused)
                .unwrap();
        }
        for event in pass.events {
            match event {
                PaneEvent::Focus(id) => {
                    layout.focus(id).unwrap();
                }
                PaneEvent::Close(id) => {
                    layout.remove(id).unwrap();
                }
                PaneEvent::Command(_) => {}
            }
        }
    }

    #[test]
    fn hiding_single_tab_bar_preserves_body_and_restores_tabs_for_multiple_tiles() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let mut probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("hide_single_tab"));
        let shown = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let body_id = probe.seen.borrow()[&TileId(1)].0;
        probe.hide_single_tab_bar = true;
        let hidden = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        assert_eq!(probe.seen.borrow()[&TileId(1)].0, body_id);
        assert_eq!(
            hidden.rects[&TileId(1)].height(),
            shown.rects[&TileId(1)].height() + 24.0
        );
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        let multiple = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        assert_eq!(multiple.rects[&TileId(1)], shown.rects[&TileId(1)]);
        assert_eq!(multiple.tab_rects.len(), 2);
        layout.remove(TileId(2)).unwrap();
        let single_again = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        assert!(single_again.tab_rects.is_empty());
        assert_eq!(single_again.rects, hidden.rects);
        assert_eq!(probe.seen.borrow()[&TileId(1)].0, body_id);
    }

    #[test]
    fn tab_bar_plus_menu_targets_the_active_tab_and_hides_with_the_bar() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let mut probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        layout.focus(TileId(2)).unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("plus"));
        let (_, output) = frame_with_output(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let plus = text_position(&output, "➕").expect("plus button in the tab bar");
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(plus, true),
        );
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(plus, false),
        );
        let (_, output) = frame_with_output(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let texts = |output: &egui::FullOutput| {
            fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            output
                .shapes
                .iter()
                .for_each(|s| collect(&s.shape, &mut out));
            out
        };
        let add = text_position(&output, "Add tab")
            .unwrap_or_else(|| panic!("open menu; visible texts: {:?}", texts(&output)));
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(add, true),
        );
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(add, false),
        );
        assert!(
            pass.events
                .iter()
                .any(|event| matches!(event, PaneEvent::Command(ProbeCommand::NewTab(TileId(2)))))
        );
        assert!(pass.edit.is_none());
        layout.remove(TileId(2)).unwrap();
        probe.hide_single_tab_bar = true;
        let (_, output) = frame_with_output(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        assert!(text_position(&output, "➕").is_none());
    }

    #[test]
    fn real_tab_click_and_close_only_change_authority_when_dispatched() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("tabs"));
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let second = pass.tab_rects[&TileId(2)];
        let pos = second.left_center() + egui::vec2(12.0, 0.0);
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(pos, true),
        );
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(pos, false),
        );
        assert_eq!(layout.focused(), Some(TileId(1)));
        assert_eq!(layout.visible_tiles(), [TileId(1)]);
        assert!(!pass.edit.as_ref().unwrap().structural);
        commit(&mut layout, pass);
        assert_eq!(layout.focused(), Some(TileId(2)));
        assert_eq!(layout.visible_tiles(), [TileId(2)]);
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let close = pass.tab_rects[&TileId(2)].right_center() - egui::vec2(14.0, 0.0);
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(close, true),
        );
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(close, false),
        );
        assert_eq!(layout.tile_order(), [TileId(1), TileId(2)]);
        assert!(
            pass.events
                .iter()
                .any(|event| matches!(event, PaneEvent::Close(TileId(2))))
        );
        commit(&mut layout, pass);
        assert_eq!(layout.tile_order(), [TileId(1)]);
    }

    #[test]
    fn panes_have_distinct_clipped_ids_and_click_focus_precedes_commands() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::Beside(TileId(1), Direction::Right))
            .unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("split"));
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let a = probe.seen.borrow()[&TileId(1)];
        let b = probe.seen.borrow()[&TileId(2)];
        assert_ne!(a.0, b.0);
        assert!(a.1.right() <= b.1.left());
        assert_eq!(a.1, pass.rects[&TileId(1)]);
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(b.1.center(), true),
        );
        let focus = pass
            .events
            .iter()
            .position(|event| matches!(event, PaneEvent::Focus(TileId(2))))
            .unwrap();
        let command = pass
            .events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    PaneEvent::Command(ProbeCommand::Pressed(TileId(2), true))
                )
            })
            .unwrap();
        assert!(focus < command);
        commit(&mut layout, pass);
        assert_eq!(layout.focused(), Some(TileId(2)));
        layout
            .move_tile(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(b.1.center(), false),
        );
        assert_eq!(probe.seen.borrow()[&TileId(2)].0, b.0);
    }

    fn commit_move_through_application(layout: &mut Layout, pass: LayoutPass<ProbeCommand>) {
        use crate::{Message, tiles::commands::WorkspaceCommand};
        let mut state = crate::SystemState::new_default_config().unwrap();
        for _ in layout.tile_order() {
            state
                .user
                .workspace
                .apply_command(
                    &mut state.workspace_runtime,
                    WorkspaceCommand::CreateTile {
                        kind: "waveform".into(),
                        placement: Placement::Edge(Direction::Right),
                        focus: false,
                    },
                )
                .unwrap();
        }
        let mut file = state.user.workspace.to_file().unwrap();
        file.layout = layout.to_file();
        state.user.workspace = crate::tiles::workspace::Workspace::from_file(file).unwrap();
        let mut edit = pass.edit.unwrap();
        edit.revision = state.user.workspace.layout().revision();
        let before = layout.root().cloned();
        state.update(Message::ApplyLayoutProposal(edit)).unwrap();
        for event in pass.events {
            match event {
                PaneEvent::Focus(id) => {
                    state
                        .update(Message::Workspace(WorkspaceCommand::FocusTile(id)))
                        .unwrap();
                }
                _ => panic!("move must emit only focus events"),
            }
        }
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.undo_stack[0].label(), "Move tile");
        let after = state.user.workspace.layout().root().cloned();
        state.update(Message::Undo(1)).unwrap();
        assert!(same_topology(
            state.user.workspace.layout().root(),
            before.as_ref()
        ));
        state.update(Message::Redo(1)).unwrap();
        assert!(same_topology(
            state.user.workspace.layout().root(),
            after.as_ref()
        ));
        *layout = state.user.workspace.layout().clone();
    }

    #[test]
    fn pointer_docking_proposes_one_structural_edit_on_release() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::Beside(TileId(1), Direction::Right))
            .unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("dock"));
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let source = pass.tab_rects[&TileId(1)].left_center() + egui::vec2(12.0, 0.0);
        let target = pass.rects[&TileId(2)].center();
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(source, true),
        );
        for pos in [source + egui::vec2(30.0, 0.0), target, target] {
            let pass = frame(
                &ctx,
                &mut adapter,
                &layout,
                &runtime,
                &probe,
                vec![egui::Event::PointerMoved(pos)],
            );
            assert!(pass.edit.is_none());
        }
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(target, false),
        );
        assert!(
            pass.edit
                .as_ref()
                .is_some_and(|edit| edit.structural && edit.moved_tile.is_some())
        );
        assert!(matches!(layout.root(), Some(LayoutNode::Split { .. })));
        commit_move_through_application(&mut layout, pass);
        assert!(
            matches!(layout.root(), Some(LayoutNode::Tabs { children, .. }) if children.len() == 2)
        );
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        assert!(pass.edit.is_none());
    }

    #[test]
    fn pointer_resize_changes_shares_without_structural_history() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::Beside(TileId(1), Direction::Right))
            .unwrap();
        let before = layout.to_file();
        let mut adapter = LayoutAdapter::new(egui::Id::new("resize"));
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let left = pass.rects[&TileId(1)];
        let right = pass.rects[&TileId(2)];
        let source = egui::pos2((left.right() + right.left()) * 0.5, left.center().y);
        let target = source + egui::vec2(80.0, 0.0);
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(source, true),
        );
        for events in [
            vec![egui::Event::PointerMoved(target)],
            pointer(target, false),
        ] {
            let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, events);
            assert!(!pass.edit.as_ref().is_some_and(|edit| edit.structural));
            commit(&mut layout, pass);
        }
        assert_ne!(before.root.as_ref(), layout.root());
        assert!(same_topology(before.root.as_ref(), layout.root()));
    }

    #[test]
    fn escape_cancels_drag_without_committing_preview_tabs() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        let before = layout.to_file();
        let mut adapter = LayoutAdapter::new(egui::Id::new("cancel"));
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let source = pass.tab_rects[&TileId(1)].left_center() + egui::vec2(12.0, 0.0);
        let target = pass.rects[&TileId(1)].right_center() - egui::vec2(5.0, 0.0);
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(source, true),
        );
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            vec![egui::Event::PointerMoved(target)],
        );
        assert!(adapter.tree.dragged_id(&ctx).is_some());
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(pass.edit.is_none());
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(target, false),
        );
        assert!(pass.edit.is_none());
        assert_eq!(layout.to_file(), before);
        assert_eq!(read_tree(&adapter.tree).unwrap(), before.root);
    }

    #[test]
    fn pointer_tab_reorder_is_structural() {
        let ctx = egui::Context::default();
        let runtime = WorkspaceRuntime::default();
        let probe = Probe::default();
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("reorder"));
        let pass = frame(&ctx, &mut adapter, &layout, &runtime, &probe, vec![]);
        let source = pass.tab_rects[&TileId(1)].left_center() + egui::vec2(12.0, 0.0);
        let target = pass.tab_rects[&TileId(2)].right_center() - egui::vec2(1.0, 0.0);
        frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(source, true),
        );
        for pos in [source + egui::vec2(30.0, 0.0), target, target] {
            frame(
                &ctx,
                &mut adapter,
                &layout,
                &runtime,
                &probe,
                vec![egui::Event::PointerMoved(pos)],
            );
        }
        let pass = frame(
            &ctx,
            &mut adapter,
            &layout,
            &runtime,
            &probe,
            pointer(target, false),
        );
        assert!(
            pass.edit
                .as_ref()
                .is_some_and(|edit| edit.structural && edit.moved_tile.is_some())
        );
        commit_move_through_application(&mut layout, pass);
        assert_eq!(layout.tile_order(), [TileId(2), TileId(1)]);
    }
    use super::*;

    #[test]
    fn adapter_round_trip_and_surviving_runtime_ids() {
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        let mut adapter = LayoutAdapter::new(egui::Id::new("test"));
        adapter.reconcile(&layout);
        assert_eq!(read_tree(&adapter.tree).unwrap().as_ref(), layout.root());
        let pane = adapter.tree.tiles.find_pane(&TileId(1)).unwrap();
        let group = adapter.tree.tiles.parent_of(pane).unwrap();
        layout.focus(TileId(2)).unwrap();
        adapter.reconcile(&layout);
        assert_eq!(adapter.tree.tiles.find_pane(&TileId(1)), Some(pane));
        assert_eq!(adapter.tree.tiles.parent_of(pane), Some(group));
        assert_eq!(read_tree(&adapter.tree).unwrap().as_ref(), layout.root());
        layout
            .insert(TileId(3), Placement::Beside(TileId(1), Direction::Down))
            .unwrap();
        adapter.reconcile(&layout);
        assert_eq!(adapter.tree.tiles.parent_of(pane), Some(group));
        assert_eq!(read_tree(&adapter.tree).unwrap().as_ref(), layout.root());
    }

    #[test]
    fn runtime_grid_and_cycles_are_errors() {
        let tree = Tree::new_grid("grid", vec![TileId(1)]);
        assert!(matches!(read_tree(&tree), Err(AdapterError::Grid)));
        let mut tree = Tree::new_tabs("cycle", vec![TileId(1)]);
        let root = tree.root.unwrap();
        tree.tiles.insert(
            root,
            Tile::Container(Container::Tabs(Tabs::new(vec![root]))),
        );
        assert!(matches!(read_tree(&tree), Err(AdapterError::InvalidTree)));
    }

    #[test]
    fn resizing_and_selection_are_not_structural() {
        let mut layout = Layout::default();
        layout.insert(TileId(1), Placement::Root).unwrap();
        layout
            .insert(TileId(2), Placement::TabAfter(TileId(1)))
            .unwrap();
        let before = layout.to_file();
        layout.focus(TileId(2)).unwrap();
        assert!(same_topology(before.root.as_ref(), layout.root()));
        layout
            .move_tile(TileId(2), Placement::Edge(Direction::Down))
            .unwrap();
        assert!(!same_topology(before.root.as_ref(), layout.root()));
        let mut after = layout.to_file();
        if let Some(LayoutNode::Split { shares, .. }) = &mut after.root {
            *shares = vec![0.2, 0.8];
        }
        assert!(same_topology(after.root.as_ref(), layout.root()));
    }
}
