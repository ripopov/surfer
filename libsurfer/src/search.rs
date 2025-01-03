use derive_more::Display;
use egui::{Button, RichText, TextEdit, Ui};
use egui_remixicon::icons as remix_icons;
use enum_iterator::Sequence;
use num::{bigint::ToBigInt, BigInt, BigUint, Num as _};
use serde::{Deserialize, Serialize};
use surfer_translation_types::VariableValue;

use crate::{
    displayed_item::{DisplayedItem, DisplayedItemIndex},
    displayed_item_tree::VisibleItemIndex,
    message::Message,
    wave_data::WaveData,
    State,
};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct SearchQuery {
    pub search_type: SearchType,
    pub search_value: BigUint,
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq, Serialize, Sequence, Default)]
pub enum SearchType {
    #[display("≠")]
    NotEqualTo,
    #[default]
    #[display("=")]
    EqualTo,
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq, Sequence, Serialize, Default)]
pub enum ConversionRadix {
    #[display("Bin")]
    Binary,
    #[display("Oct")]
    Octal,
    #[default]
    #[display("Dec")]
    Decimal,
    #[display("Hex")]
    Hexadecimal,
}

impl Into<u32> for ConversionRadix {
    fn into(self) -> u32 {
        match self {
            ConversionRadix::Binary => 2,
            ConversionRadix::Octal => 8,
            ConversionRadix::Decimal => 10,
            ConversionRadix::Hexadecimal => 16,
        }
    }
}

impl WaveData {
    /// Set cursor at next (or previous, if `next` is false) transition of `variable`. If `skip_zero` is true,
    /// use the next transition to a non-zero value.
    pub fn set_cursor_at_transition(
        &mut self,
        next: bool,
        variable: Option<DisplayedItemIndex>,
        search_query: Option<SearchQuery>,
    ) {
        if let Some(DisplayedItemIndex(vidx)) = variable.or(self.focused_item) {
            if let Some(cursor) = &self.cursor {
                if let Some(DisplayedItem::Variable(variable)) = &self
                    .items_tree
                    .get_visible(VisibleItemIndex(vidx))
                    .and_then(|node| self.displayed_items.get(&node.item))
                {
                    if let Some(waves) = self.inner.as_waves() {
                        let num_timestamps = &self
                            .num_timestamps()
                            .expect("No timestamp count even though waveforms should be loaded");
                        if let Some(cursor) = find_transition_time(
                            next,
                            search_query,
                            waves,
                            variable,
                            cursor,
                            num_timestamps,
                        ) {
                            self.cursor = Some(cursor);
                        }
                    }
                }
            }
        }
    }
}

fn find_transition_time(
    next: bool,
    search_query: Option<SearchQuery>,
    waves: &crate::wave_container::WaveContainer,
    variable: &crate::displayed_item::DisplayedVariable,
    cursor: &BigInt,
    num_timestamps: &BigInt,
) -> Option<BigInt> {
    let mut new_cursor = cursor.clone();
    if let Ok(Some(res)) = waves.query_variable(
        &variable.variable_ref,
        &cursor.to_biguint().unwrap_or_default(),
    ) {
        if next {
            if let Some(ref time) = res.next {
                if let Some(stime) = &time.to_bigint() {
                    new_cursor.clone_from(stime);
                }
            } else {
                // No next transition, go to end
                new_cursor.clone_from(num_timestamps);
                return Some(new_cursor);
            }
        } else if let Some(stime) = &res.current.unwrap().0.to_bigint() {
            let bigone = &BigInt::from(1);
            // Check if we are on a transition
            if stime == cursor && cursor >= bigone {
                // If so, subtract cursor position by one
                if let Ok(Some(newres)) = waves.query_variable(
                    &variable.variable_ref,
                    &(cursor - bigone).to_biguint().unwrap_or_default(),
                ) {
                    if let Some(current) = newres.current {
                        if let Some(newstime) = current.0.to_bigint() {
                            new_cursor.clone_from(&newstime);
                        }
                    }
                }
            } else {
                new_cursor.clone_from(stime);
                return Some(new_cursor);
            }
        }

        if let Some(query) = search_query {
            match query {
                SearchQuery {
                    search_type: SearchType::NotEqualTo,
                    search_value,
                } => {
                    // check if the next transition is 0, if so and requested, go to
                    // next positive transition
                    let next_value = waves.query_variable(
                        &variable.variable_ref,
                        &new_cursor.to_biguint().unwrap_or_default(),
                    );
                    if next_value.is_ok_and(|r| {
                        r.is_some_and(|r| {
                            r.current.is_some_and(|v| match v.1 {
                                VariableValue::BigUint(v) => v == search_value,
                                _ => false,
                            })
                        })
                    }) {
                        if let Some(cursor) = find_transition_time(
                            next,
                            None,
                            waves,
                            variable,
                            &new_cursor,
                            num_timestamps,
                        ) {
                            new_cursor.clone_from(&cursor);
                        };
                    }
                }
                SearchQuery {
                    search_type: SearchType::EqualTo,
                    search_value,
                } => {
                    // find transition where value is zero
                    let next_value = waves.query_variable(
                        &variable.variable_ref,
                        &new_cursor.to_biguint().unwrap_or_default(),
                    );
                    if next_value.is_ok_and(|r| {
                        r.is_some_and(|r| {
                            r.current.is_some_and(|v| match v.1 {
                                VariableValue::BigUint(val) => {
                                    if val == search_value {
                                        new_cursor = v.0.into();
                                        false
                                    } else {
                                        true
                                    }
                                }
                                _ => true,
                            })
                        })
                    }) {
                        if let Some(cursor) = find_transition_time(
                            next,
                            Some(SearchQuery {
                                search_type: SearchType::EqualTo,
                                search_value,
                            }),
                            waves,
                            variable,
                            &new_cursor,
                            num_timestamps,
                        ) {
                            new_cursor.clone_from(&cursor);
                        }
                    }
                }
            }
        }
    }
    Some(new_cursor)
}

impl State {
    pub fn draw_search_widget(
        &self,
        msgs: &mut Vec<Message>,
        item_selected: bool,
        cursor_set: bool,
        ui: &mut Ui,
    ) {
        // Create text edit
        let response = ui.add(
            TextEdit::singleline(&mut *self.sys.search_value.borrow_mut())
                .desired_width(100.0)
                .clip_text(true)
                .hint_text("Value"),
        );
        // Handle focus of text edit
        if response.gained_focus() {
            msgs.push(Message::SetSearchValueFocused(true));
        }
        if response.lost_focus() {
            msgs.push(Message::SetSearchValueFocused(false));
        }
        ui.menu_button(self.search_type.to_string(), |ui| {
            for search_type in enum_iterator::all::<SearchType>() {
                ui.radio(self.search_type == search_type, search_type.to_string())
                    .clicked()
                    .then(|| {
                        ui.close_menu();
                        msgs.push(Message::SetSearchType(search_type));
                    });
            }
        });
        ui.menu_button(self.search_radix.to_string(), |ui| {
            for radix in enum_iterator::all::<ConversionRadix>() {
                ui.radio(self.search_radix == radix, radix.to_string())
                    .clicked()
                    .then(|| {
                        ui.close_menu();
                        msgs.push(Message::SetSearchRadix(radix));
                    });
            }
        });
        let button =
            Button::new(RichText::new(remix_icons::CONTRACT_LEFT_FILL).heading()).frame(false);
        ui.add_enabled(item_selected && cursor_set, button)
            .on_hover_text("Go to previous time with value on focused variable")
            .clicked()
            .then(|| {
                self.find_next_transition(msgs, false);
            });

        let button =
            Button::new(RichText::new(remix_icons::CONTRACT_RIGHT_FILL).heading()).frame(false);
        ui.add_enabled(item_selected && cursor_set, button)
            .on_hover_text("Go to next time with value on focused variable")
            .clicked()
            .then(|| {
                self.find_next_transition(msgs, true);
            });
    }

    fn find_next_transition(&self, msgs: &mut Vec<Message>, next: bool) {
        if let Ok(val) = BigUint::from_str_radix(
            (*self.sys.search_value.borrow()).as_str(),
            self.search_radix.clone().into(),
        ) {
            msgs.push(Message::MoveCursorToTransition {
                next,
                variable: None,
                search_query: Some(SearchQuery {
                    search_type: self.search_type.clone(),
                    search_value: val,
                }),
            })
        }
    }
}
