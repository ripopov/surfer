use derive_more::Display;
use egui::{Button, RichText, TextEdit, Ui};
use egui_remixicon::icons as remix_icons;
use enum_iterator::Sequence;
use num::{bigint::ToBigInt, BigInt, BigUint, Num as _};
use serde::{Deserialize, Serialize};
use surfer_translation_types::VariableValue;

use crate::{
    displayed_item::DisplayedItem, displayed_item_tree::VisibleItemIndex, message::Message,
    wave_data::WaveData, State,
};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub enum SearchQuery {
    Numeric {
        query_type: QueryType,
        query_value: BigUint,
    },
    Translated {
        query_type: QueryTextType,
        query_string: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Display, PartialEq, Serialize, Sequence, Default)]
pub enum QueryType {
    #[display(">")]
    GreaterThan,
    #[display("≥")]
    GreaterThanEqualTo,
    #[display("<")]
    LessThan,
    #[display("≤")]
    LessThanEqualTo,
    #[display("≠")]
    NotEqualTo,
    #[default]
    #[display("=")]
    EqualTo,
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq, Sequence, Serialize, Default)]
pub enum QueryRadix {
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

impl From<QueryRadix> for u32 {
    fn from(val: QueryRadix) -> Self {
        match val {
            QueryRadix::Binary => 2,
            QueryRadix::Octal => 8,
            QueryRadix::Decimal => 10,
            QueryRadix::Hexadecimal => 16,
        }
    }
}

#[derive(Clone, Debug, Display, PartialEq, Serialize, Deserialize, Sequence)]
pub enum QueryTextType {
    #[display("Regular expression")]
    Regex,

    #[display("Value starts with")]
    Start,

    #[display("Value contains")]
    Contain,
}

impl WaveData {
    /// Set cursor at next (or previous, if `next` is false) transition of `variable`. If `skip_zero` is true,
    /// use the next transition to a non-zero value.
    pub fn set_cursor_at_transition(
        &mut self,
        next: bool,
        variable: Option<VisibleItemIndex>,
        search_query: Option<SearchQuery>,
    ) {
        if let Some(VisibleItemIndex(vidx)) = variable.or(self.focused_item) {
            if let Some(cursor) = &self.cursor {
                if let Some(DisplayedItem::Variable(variable)) = &self
                    .items_tree
                    .get_visible(VisibleItemIndex(vidx))
                    .and_then(|node| self.displayed_items.get(&node.item_ref))
                {
                    if let Some(waves) = self.inner.as_waves() {
                        let num_timestamps = &self
                            .num_timestamps()
                            .expect("No timestamp count even though waveforms should be loaded");
                        if let Some(cursor) = match search_query {
                            Some(SearchQuery::Numeric {
                                query_type,
                                query_value,
                            }) => find_transition_time_numeric(
                                next,
                                query_type,
                                &query_value,
                                waves,
                                variable,
                                cursor,
                                num_timestamps,
                            ),
                            Some(SearchQuery::Translated {
                                query_type,
                                query_string,
                            }) => find_transition_time_translated(
                                next,
                                query_type,
                                query_string,
                                waves,
                                variable,
                                cursor,
                                num_timestamps,
                            ),
                            None => find_transition_time_any(next, waves, variable, cursor),
                        } {
                            self.cursor = cursor.to_bigint();
                        }
                    }
                }
            }
        }
    }
}

fn numeric_compare(query_type: QueryType, query_value: &BigUint, current_value: &BigUint) -> bool {
    match query_type {
        QueryType::EqualTo => current_value == query_value,
        QueryType::NotEqualTo => current_value != query_value,
        QueryType::GreaterThan => current_value > query_value,
        QueryType::GreaterThanEqualTo => current_value >= query_value,
        QueryType::LessThan => current_value < query_value,
        QueryType::LessThanEqualTo => current_value <= query_value,
    }
}

fn find_transition_time_numeric(
    next: bool,
    query_type: QueryType,
    query_value: &BigUint,
    waves: &crate::wave_container::WaveContainer,
    variable: &crate::displayed_item::DisplayedVariable,
    cursor: &BigInt,
    num_timestamps: &BigInt,
) -> Option<BigUint> {
    let mut new_cursor = cursor.to_biguint().unwrap_or_default();
    let big_one = &BigUint::from(1u8);

    loop {
        if let Ok(Some(res)) = waves.query_variable(&variable.variable_ref, &new_cursor) {
            if let Some((time, val)) = res.current {
                match val {
                    VariableValue::BigUint(current_value) => {
                        if numeric_compare(query_type, query_value, &current_value) {
                            if time.to_bigint() == Some(num_timestamps.clone()) {
                                return None;
                            } else {
                                if next {
                                    return res.next;
                                } else {
                                    if time == new_cursor {
                                        // On an edge
                                        if &time >= big_one {
                                            if let Ok(Some(res)) = waves.query_variable(
                                                &variable.variable_ref,
                                                &(time - big_one),
                                            ) {
                                                if let Some((time, _)) = res.current {
                                                    return Some(time);
                                                } else {
                                                    return Some(0u8.into());
                                                }
                                            }
                                        }
                                    } else {
                                        return Some(time);
                                    }
                                }
                            }
                        } else {
                            if next {
                                if let Some(next_time) = res.next {
                                    new_cursor.clone_from(&next_time);
                                } else {
                                    return None;
                                }
                            } else {
                                if &time >= big_one {
                                    new_cursor.clone_from(&(time - big_one))
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    VariableValue::String(_) => break,
                }
            }
        }
    }
    return None;
}

fn find_transition_time_any(
    next: bool,
    waves: &crate::wave_container::WaveContainer,
    variable: &crate::displayed_item::DisplayedVariable,
    cursor: &BigInt,
) -> Option<BigUint> {
    let new_cursor = cursor.to_biguint().unwrap_or_default();
    let big_one = &BigUint::from(1u8);
    // Any transition
    if let Ok(Some(res)) = waves.query_variable(&variable.variable_ref, &new_cursor) {
        if let Some((time, _)) = res.current {
            if next {
                return res.next;
            } else {
                if time == new_cursor {
                    // On an edge
                    if &time >= big_one {
                        if let Ok(Some(res)) =
                            waves.query_variable(&variable.variable_ref, &(time - big_one))
                        {
                            if let Some((time, _)) = res.current {
                                return Some(time);
                            } else {
                                return Some(0u8.into());
                            }
                        }
                    }
                } else {
                    return Some(time);
                }
            }
        }
    }
    return None;
}

fn find_transition_time_translated(
    _next: bool,
    _query_type: QueryTextType,
    _query_string: String,
    _waves: &crate::wave_container::WaveContainer,
    _variable: &crate::displayed_item::DisplayedVariable,
    _cursor: &BigInt,
    _num_timestamps: &BigInt,
) -> Option<BigUint> {
    return None;
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
            msgs.push(Message::SetQueryValueFocused(true));
        }
        if response.lost_focus() {
            msgs.push(Message::SetQueryValueFocused(false));
        }
        ui.add(Button::new("#").selected(self.query_numerical_value))
            .on_hover_text("Query numerical value")
            .clicked()
            .then(|| {
                msgs.push(Message::SetQueryNumericalValue(!self.query_numerical_value));
            });

        if self.query_numerical_value {
            ui.menu_button(self.query_type.to_string(), |ui| {
                for search_type in enum_iterator::all::<QueryType>() {
                    ui.radio(self.query_type == search_type, search_type.to_string())
                        .clicked()
                        .then(|| {
                            ui.close_menu();
                            msgs.push(Message::SetQueryType(search_type));
                        });
                }
            });
            ui.menu_button(self.query_radix.to_string(), |ui| {
                for radix in enum_iterator::all::<QueryRadix>() {
                    ui.radio(self.query_radix == radix, radix.to_string())
                        .clicked()
                        .then(|| {
                            ui.close_menu();
                            msgs.push(Message::SetQueryRadix(radix));
                        });
                }
            });
        } else {
            ui.menu_button(remix_icons::FILTER_FILL, |ui| {
                for text_type in enum_iterator::all::<QueryTextType>() {
                    ui.radio(self.query_text_type == text_type, text_type.to_string())
                        .clicked()
                        .then(|| {
                            ui.close_menu();
                            msgs.push(Message::SetQueryTextType(text_type));
                        });
                }
            });
        }
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
        if self.query_numerical_value {
            if let Ok(val) = BigUint::from_str_radix(
                (*self.sys.search_value.borrow()).as_str(),
                self.query_radix.clone().into(),
            ) {
                msgs.push(Message::MoveCursorToTransition {
                    next,
                    variable: None,
                    search_query: Some(SearchQuery::Numeric {
                        query_type: self.query_type,
                        query_value: val,
                    }),
                });
            }
        } else {
            msgs.push(Message::MoveCursorToTransition {
                next,
                variable: None,
                search_query: Some(SearchQuery::Translated {
                    query_type: self.query_text_type.clone(),
                    query_string: (*self.sys.search_value.borrow()).to_string(),
                }),
            });
        }
    }
}
