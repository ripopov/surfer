use derive_more::{Display, FromStr};
use enum_iterator::Sequence;
use itertools::Itertools;

use serde::{Deserialize, Serialize};

use crate::displayed_item_tree::Node;
use crate::wave_container::{ScopeRefExt, VariableRefExt};
use crate::{displayed_item::DisplayedItem, wave_container::VariableRef, wave_data::WaveData};

#[derive(PartialEq, Copy, Clone, Debug, Deserialize, Display, FromStr, Serialize, Sequence)]
pub enum VariableNameType {
    /// Local variable name only (i.e. for tb.dut.clk => clk)
    Local,

    /// Add unique prefix, prefix + local
    Unique,

    /// Full variable name (i.e. tb.dut.clk => tb.dut.clk)
    Global,
}

impl WaveData {
    pub fn compute_variable_display_names(&mut self) {
        // First pass: collect all unique variable refs
        let full_names: Vec<VariableRef> = self
            .items_tree
            .iter()
            .filter_map(|node| {
                self.displayed_items
                    .get(&node.item_ref)
                    .and_then(|item| match item {
                        DisplayedItem::Variable(variable) => Some(variable.variable_ref.clone()),
                        _ => None,
                    })
            })
            .unique()
            .collect();

        // Single pass: update display names for all items
        for Node { item_ref, .. } in self.items_tree.iter() {
            self.displayed_items
                .entry(*item_ref)
                .and_modify(|item| match item {
                    DisplayedItem::Variable(variable) => {
                        let local_name = variable.variable_ref.name.clone();
                        variable.display_name = match variable.display_name_type {
                            VariableNameType::Local => local_name,
                            VariableNameType::Global => variable.variable_ref.full_path_string(),
                            VariableNameType::Unique => {
                                compute_unique_variable_name(&variable.variable_ref, &full_names)
                            }
                        };
                        if self.display_variable_indices {
                            let index = self
                                .inner
                                .as_waves()
                                .unwrap()
                                .variable_meta(&variable.variable_ref)
                                .ok()
                                .as_ref()
                                .and_then(|meta| meta.index)
                                .map(|index| format!(" {index}"))
                                .unwrap_or_default();
                            variable.display_name = format!("{}{}", variable.display_name, index);
                        }
                    }
                    DisplayedItem::Divider(_) => {}
                    DisplayedItem::Marker(_) => {}
                    DisplayedItem::TimeLine(_) => {}
                    DisplayedItem::Placeholder(_) => {}
                    DisplayedItem::Stream(_) => {}
                    DisplayedItem::Group(_) => {}
                });
        }
    }

    pub fn force_variable_name_type(&mut self, name_type: VariableNameType) {
        for Node { item_ref, .. } in self.items_tree.iter() {
            self.displayed_items.entry(*item_ref).and_modify(|item| {
                if let DisplayedItem::Variable(variable) = item {
                    variable.display_name_type = name_type;
                }
            });
        }
        self.default_variable_name_type = name_type;
        self.compute_variable_display_names();
    }
}

/// Compute a minimal unique variable name by adding scope components until
/// the name becomes unique among the given set of variables.
fn compute_unique_variable_name(variable: &VariableRef, all_variables: &[VariableRef]) -> String {
    let other_variables: Vec<_> = all_variables
        .iter()
        .filter(|&v| v.full_path_string() != variable.full_path_string())
        .collect();

    let mut scope_depth = 0;
    loop {
        let current_name = format_variable_name(variable, scope_depth);

        // Check if this name is unique
        if !other_variables
            .iter()
            .any(|v| format_variable_name(v, scope_depth) == current_name)
        {
            return current_name;
        }

        scope_depth += 1;
    }
}

/// Format a variable name with the given scope depth.
/// scope_depth=0 returns just the local name.
/// Higher values add scope components from the end of the path.
fn format_variable_name(variable: &VariableRef, scope_depth: usize) -> String {
    if scope_depth == 0 {
        variable.name.clone()
    } else {
        let path_parts = variable.path.strs();
        let ellipsis = if scope_depth < path_parts.len() {
            "…"
        } else {
            ""
        };
        let scope = path_parts.iter().rev().take(scope_depth).rev().join(".");
        format!("{}{}.{}", ellipsis, scope, variable.name)
    }
}
