use egui::{Button, Context, Pos2, RichText, Window};
use egui_graphs::{
    to_graph, DefaultEdgeShape, DefaultNodeShape, FruchtermanReingold, FruchtermanReingoldState,
    Graph, LayoutForceDirected,
};
use egui_plot::{Bar, BarChart};
use egui_remixicon::icons;
use num::{BigUint, ToPrimitive};
use petgraph::{
    csr::DefaultIx,
    graph::{EdgeIndex, NodeIndex},
    prelude::StableGraph,
    Directed,
};
use serde::Deserialize;
use std::collections::HashMap;
use surfer_translation_types::{Translator, ValueRepr, VariableValue};

use crate::{
    displayed_item::{DisplayedItem, DisplayedVariable},
    displayed_item_tree::VisibleItemIndex,
    message::Message,
    translation::AnyTranslator,
    wave_container::{QueryResult, VariableMeta},
    SystemState,
};

// Ensure getrandom is included for wasm32 targets
#[cfg(target_arch = "wasm32")]
use getrandom as _;

type NodeMapping = HashMap<VariableValue, NodeIndex>;
type EdgeMapping = HashMap<(VariableValue, VariableValue), EdgeIndex>;
type GraphType = Graph<(), (), Directed, DefaultIx, DefaultNodeShape, DefaultEdgeShape>;
pub type AnalysisWindowVisibilityMap = HashMap<AnalysisWindow, bool>;

pub struct VariableStatistics {
    pub state_times: Vec<(VariableValue, BigUint)>,
    pub variable: DisplayedVariable,
    pub graph: GraphType,
    pub nodemapping: NodeMapping,
    pub edgemapping: EdgeMapping,
    pub bars: Vec<Bar>,
    pub graph_autozoom: bool,
    pub graph_autoplace: bool,
    pub graph_firstrun: bool,
}

#[derive(Debug, Deserialize, Hash, Eq, PartialEq)]
pub enum AnalysisWindow {
    State,
    Histogram,
}

impl AnalysisWindow {
    /// Returns a HashMap with all variants initialized to false
    pub fn default_visibility_map() -> AnalysisWindowVisibilityMap {
        let mut map = HashMap::new();
        map.insert(AnalysisWindow::State, false);
        map.insert(AnalysisWindow::Histogram, false);
        map
    }
}

pub fn compute_signal_transition_stats(
    data: Vec<(BigUint, VariableValue)>,
) -> (
    HashMap<(VariableValue, VariableValue), usize>,
    HashMap<VariableValue, BigUint>,
) {
    let mut transition_counts: HashMap<(VariableValue, VariableValue), usize> = HashMap::new();
    let mut state_times: HashMap<VariableValue, BigUint> = HashMap::new();

    // To track previous (time, value)
    let mut prev: Option<(BigUint, VariableValue)> = None;

    for (time, value) in data {
        if let Some((prev_time, prev_value)) = &prev {
            let dt = time.clone() - prev_time.clone();
            *state_times
                .entry(prev_value.clone())
                .or_insert(BigUint::ZERO) += dt;

            if *prev_value != value {
                *transition_counts
                    .entry((prev_value.clone(), value.clone()))
                    .or_insert(0) += 1;
            }
        }
        prev = Some((time, value));
    }

    (transition_counts, state_times)
}

fn get_state_graph(
    transition_counts: &HashMap<(VariableValue, VariableValue), usize>,
    state_times: &[(VariableValue, BigUint)],
    meta: &VariableMeta,
    translator: &AnyTranslator,
) -> (GraphType, NodeMapping, EdgeMapping) {
    let mut nodemapping: NodeMapping = HashMap::new();
    let mut edgemapping: EdgeMapping = HashMap::new();
    let g: StableGraph<(), (), Directed, DefaultIx> = StableGraph::new();
    let mut graph: GraphType = to_graph(&g);
    let cols = state_times.len().isqrt().max(1); // avoid div by zero
    for (idx, (node, _)) in state_times.iter().enumerate() {
        // Use the node's string representation as the label
        let label = get_translated_value(node, translator, meta);

        nodemapping.insert(
            node.clone(),
            graph.add_node_with_label_and_location(
                (),
                label,
                Pos2::new((idx / cols) as f32, (idx % cols) as f32),
            ),
        );
    }
    for ((start, end), count) in transition_counts.iter() {
        let start_node = nodemapping[start];
        let end_node = nodemapping[end];
        edgemapping.insert(
            (start.clone(), end.clone()),
            graph.add_edge_with_label(start_node, end_node, (), count.to_string()),
        );
    }
    (graph, nodemapping, edgemapping)
}

impl SystemState {
    pub(crate) fn handle_open_analysis_window(
        &mut self,
        vidx: Option<VisibleItemIndex>,
        mode: AnalysisWindow,
    ) -> Option<()> {
        let waves = self.user.waves.as_mut()?;
        let vidx = vidx.or(waves.focused_item)?;
        let item_index = waves.items_tree.to_displayed(vidx)?;
        let node = waves.items_tree.get(item_index)?;
        let item = waves.displayed_items.get(&node.item_ref)?;
        match item {
            DisplayedItem::Variable(v) => {
                let data = waves
                    .inner
                    .as_waves()
                    .unwrap()
                    .time_value_vector(&v.variable_ref);
                let (transitions, state_times) =
                    crate::analysis::compute_signal_transition_stats(data);
                let mut state_times_vec: Vec<(VariableValue, BigUint)> =
                    state_times.into_iter().collect();
                state_times_vec.sort_by(|x, y| x.0.cmp(&y.0));
                let translator = self.translators.get_translator(
                    &(v.format.clone().unwrap_or(self.translators.default.clone())),
                );
                let mut bars: Vec<Bar> = vec![];
                let total_time = waves
                    .num_timestamps()
                    .unwrap_or_default()
                    .to_f64()
                    .unwrap_or_default()
                    .max(1.0);
                let meta = waves
                    .inner
                    .as_waves()
                    .unwrap()
                    .variable_meta(&v.variable_ref)
                    .unwrap();
                for (i, (val, time)) in state_times_vec.iter().enumerate() {
                    let name = get_translated_value(val, translator, &meta);
                    bars.push(
                        Bar::new(i as f64, 100. * time.to_f64().unwrap() / total_time).name(name),
                    );
                }

                let (graph, nodemapping, edgemapping) =
                    get_state_graph(&transitions, &state_times_vec, &meta, translator);
                self.analysis_statistics = Some(VariableStatistics {
                    // transition_counts: transitions.clone(),
                    state_times: state_times_vec.clone(),
                    variable: v.clone(),
                    graph,
                    nodemapping,
                    edgemapping,
                    bars,
                    graph_autoplace: true,
                    graph_autozoom: true,
                    graph_firstrun: true,
                });
                self.show_analysis_window.insert(mode, true);
            }
            _ => tracing::error!("Cannot open analysis window for anything other than variable!"),
        }
        Some(())
    }

    pub fn draw_histogram_window(&mut self, ctx: &Context, msgs: &mut Vec<Message>) {
        let mut open = true;
        if let Some(stats) = self.analysis_statistics.as_ref() {
            let var_name = stats.variable.variable_ref.full_path().join(".");
            Window::new(format!("Value histogram - {var_name}"))
                .collapsible(true)
                .resizable(true)
                .default_width(400.)
                .default_height(400.)
                .open(&mut open)
                .show(ctx, |ui| {
                    egui_plot::Plot::new(format!("state_histogram_{var_name}"))
                        .include_y(0.0)
                        .x_axis_formatter(|mark, _| {
                            stats
                                .state_times
                                .get(mark.value as usize)
                                .map(|(v, _)| v.to_string())
                                .unwrap_or_default()
                        })
                        .y_axis_formatter(|mark, _| format!("{}%", mark.value))
                        .label_formatter(|name, value| {
                            if !name.is_empty() {
                                format!("State: {}\n{}%", name, value.y)
                            } else {
                                String::new()
                            }
                        })
                        .show(ui, |plot_ui| {
                            plot_ui.bar_chart(BarChart::new("Time in state", stats.bars.clone()));
                        });
                });
        }

        if !open {
            msgs.push(Message::SetAnalysisWindowVisible(
                AnalysisWindow::Histogram,
                false,
            ));
        }
    }

    pub fn draw_state_window(&mut self, ctx: &Context, msgs: &mut Vec<Message>) {
        if let Some(waves) = self.user.waves.as_ref() {
            let mut open = true;
            let var_name = self
                .analysis_statistics
                .as_ref()
                .map_or(String::new(), |stats| {
                    stats.variable.variable_ref.full_path().join(".")
                });
            Window::new(format!("State transitions - {var_name}"))
                .collapsible(true)
                .resizable(true)
                //.default_width(400.)
                //.default_height(400.)
                .open(&mut open)
                .show(ctx, |ui| {
                    if let Some(stats) = self.analysis_statistics.as_mut() {
                        ui.horizontal(|ui| {
                            let button =
                                Button::new(RichText::new(icons::ASPECT_RATIO_FILL).heading())
                                    .frame(false);
                            ui.add_enabled(true, button)
                                .on_hover_text("Zoom to fit")
                                .clicked()
                                .then(|| stats.graph_autozoom = true);
                            let button =
                                Button::new(RichText::new(icons::PLAY_LARGE_FILL).heading())
                                    .frame(false);
                            ui.add_enabled(true, button)
                                .on_hover_text("Auto-placement")
                                .clicked()
                                .then(|| stats.graph_autoplace = true);
                        });
                        let (current_value, previous_value) =
                            get_variable_values_at_cursor(stats, waves);
                        // Update node colors based on cursor
                        for (node_value, nidx) in stats.nodemapping.iter() {
                            if let Some(node) = stats.graph.node_mut(*nidx) {
                                if let Some(color) =
                                    get_node_color(node_value, &current_value, &previous_value)
                                {
                                    node.set_color(color);
                                } else {
                                    node.set_color(ecolor::Color32::WHITE);
                                    // node.reset_color();
                                }
                            }
                        }

                        // Update edge colors (currently not working)
                        for (edge_values, eidx) in stats.edgemapping.iter() {
                            if let Some(edge) = stats.graph.edge_mut(*eidx) {
                                if let Some(_color) =
                                    get_edge_color(edge_values, &current_value, &previous_value)
                                {
                                    edge.set_selected(true);
                                    //edge.set_color(color);
                                } else {
                                    edge.set_selected(false);
                                    //edge.reset_color();
                                }
                            }
                        }

                        // Create graph widget
                        let settings_interaction = &egui_graphs::SettingsInteraction::new()
                            .with_node_selection_enabled(true)
                            .with_dragging_enabled(true)
                            .with_node_clicking_enabled(true)
                            .with_edge_selection_enabled(true);
                        let settings_navigation = &egui_graphs::SettingsNavigation::new()
                            .with_zoom_and_pan_enabled(true)
                            .with_fit_to_screen_enabled(stats.graph_autozoom);

                        let mut view = egui_graphs::GraphView::<
                            _,
                            _,
                            _,
                            _,
                            _,
                            _,
                            FruchtermanReingoldState,
                            LayoutForceDirected<FruchtermanReingold>,
                        >::new(&mut stats.graph)
                        .with_styles(
                            &egui_graphs::SettingsStyle::default().with_labels_always(true),
                        )
                        // .with_styles(settings_style)
                        .with_interactions(settings_interaction)
                        .with_navigations(settings_navigation);
                        let mut state =
                            egui_graphs::get_layout_state::<FruchtermanReingoldState>(ui, None);
                        state.is_running =
                            stats.graph_autoplace || stats.graph_autozoom || stats.graph_firstrun;
                        egui_graphs::set_layout_state::<FruchtermanReingoldState>(ui, state, None);
                        stats.graph_autozoom = false;
                        ui.add(&mut view);
                    }

                    if let Some(stats) = self.analysis_statistics.as_mut() {
                        if stats.graph_autoplace {
                            egui_graphs::GraphView::<
                                _,
                                _,
                                _,
                                _,
                                _,
                                _,
                                FruchtermanReingoldState,
                                LayoutForceDirected<FruchtermanReingold>,
                            >::fast_forward_budgeted(
                                ui, &mut stats.graph, 1000, 1000, None
                            );
                            stats.graph_autoplace = false;
                        }
                        if stats.graph_firstrun {
                            stats.graph_autozoom = true;
                            stats.graph_firstrun = false;
                        }
                    }
                });

            if !open {
                msgs.push(Message::SetAnalysisWindowVisible(
                    AnalysisWindow::State,
                    false,
                ));
            }
        }
    }
}

fn get_variable_values_at_cursor(
    stats: &mut VariableStatistics,
    waves: &crate::wave_data::WaveData,
) -> (Option<VariableValue>, Option<VariableValue>) {
    if let Some(cursor) = waves.cursor.as_ref().and_then(|n| n.to_biguint()) {
        // Check value at cursor
        let QueryResult { current, .. } = waves
            .inner
            .as_waves()
            .unwrap()
            .query_variable(&stats.variable.variable_ref, &cursor)
            .unwrap()
            .unwrap();
        if let Some((last_time, val)) = current {
            // If at a transition
            if last_time == cursor && last_time > BigUint::ZERO {
                // Check one time unit before
                let QueryResult { current, .. } = waves
                    .inner
                    .as_waves()
                    .unwrap()
                    .query_variable(&stats.variable.variable_ref, &(cursor - 1u8))
                    .unwrap()
                    .unwrap();
                if let Some((_, prev_val)) = current {
                    return (Some(val), Some(prev_val));
                }
            }
            return (Some(val), None);
        }
    }
    (None, None)
}

#[inline]
fn get_edge_color(
    edge_values: &(VariableValue, VariableValue),
    current_value: &Option<VariableValue>,
    previous_value: &Option<VariableValue>,
) -> Option<ecolor::Color32> {
    if let Some(prev) = previous_value.as_ref() {
        if let Some(curr) = current_value.as_ref() {
            if edge_values == &(prev.clone(), curr.clone()) {
                return Some(ecolor::Color32::RED);
            }
        }
    }
    None
}

#[inline]
fn get_node_color(
    cursor_value: &VariableValue,
    current_value: &Option<VariableValue>,
    previous_value: &Option<VariableValue>,
) -> Option<ecolor::Color32> {
    if Some(cursor_value) == current_value.as_ref() || Some(cursor_value) == previous_value.as_ref()
    {
        Some(ecolor::Color32::RED)
    } else {
        None
    }
}

fn get_translated_value(
    value: &VariableValue,
    translator: &AnyTranslator,
    meta: &VariableMeta,
) -> String {
    if let Ok(translated) = translator.translate(meta, value) {
        match translated.val {
            ValueRepr::Bit(c) => c.to_string(),
            ValueRepr::Bits(_, s) => s,
            ValueRepr::String(s) => s,
            _ => value.to_string(),
        }
    } else {
        value.to_string()
    }
}
