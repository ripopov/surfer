use egui::{Button, RichText, TextEdit, Ui};
use egui_remixicon::icons as remix_icons;
use num::{bigint::ToBigInt, BigInt, BigUint, Num as _};
use serde::Deserialize;
use surfer_translation_types::VariableValue;

use crate::{
    displayed_item::{DisplayedItem, DisplayedItemIndex},
    message::Message,
    wave_data::WaveData,
    State,
};

#[derive(Debug, Deserialize, PartialEq)]
pub enum TransitionType {
    Any,
    NotEqualTo(BigUint),
    EqualTo(BigUint),
}

impl WaveData {
    /// Set cursor at next (or previous, if `next` is false) transition of `variable`. If `skip_zero` is true,
    /// use the next transition to a non-zero value.
    pub fn set_cursor_at_transition(
        &mut self,
        next: bool,
        variable: Option<DisplayedItemIndex>,
        transition_type: TransitionType,
    ) {
        if let Some(DisplayedItemIndex(vidx)) = variable.or(self.focused_item) {
            if let Some(cursor) = &self.cursor {
                if let Some(DisplayedItem::Variable(variable)) = &self
                    .displayed_items_order
                    .get(vidx)
                    .and_then(|id| self.displayed_items.get(id))
                {
                    if let Some(waves) = self.inner.as_waves() {
                        let num_timestamps = &self
                            .num_timestamps()
                            .expect("No timestamp count even though waveforms should be loaded");
                        if let Some(cursor) = find_transition_time(
                            next,
                            transition_type,
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
    transition_type: TransitionType,
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

        match transition_type {
            TransitionType::NotEqualTo(n) => {
                // check if the next transition is 0, if so and requested, go to
                // next positive transition
                let next_value = waves.query_variable(
                    &variable.variable_ref,
                    &new_cursor.to_biguint().unwrap_or_default(),
                );
                if next_value.is_ok_and(|r| {
                    r.is_some_and(|r| {
                        r.current.is_some_and(|v| match v.1 {
                            VariableValue::BigUint(v) => v == n,
                            _ => false,
                        })
                    })
                }) {
                    if let Some(cursor) = find_transition_time(
                        next,
                        TransitionType::Any,
                        waves,
                        variable,
                        &new_cursor,
                        num_timestamps,
                    ) {
                        new_cursor.clone_from(&cursor);
                    };
                }
            }
            TransitionType::EqualTo(n) => {
                // find transition where value is zero
                let next_value = waves.query_variable(
                    &variable.variable_ref,
                    &new_cursor.to_biguint().unwrap_or_default(),
                );
                if next_value.is_ok_and(|r| {
                    r.is_some_and(|r| {
                        r.current.is_some_and(|v| match v.1 {
                            VariableValue::BigUint(val) => {
                                if val == n {
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
                        TransitionType::EqualTo(n),
                        waves,
                        variable,
                        &new_cursor,
                        num_timestamps,
                    ) {
                        new_cursor.clone_from(&cursor);
                    }
                }
            }
            TransitionType::Any => {}
        }
    }
    Some(new_cursor)
}

impl State {
    pub fn draw_find_widget(
        &self,
        msgs: &mut Vec<Message>,
        item_selected: bool,
        cursor_set: bool,
        ui: &mut Ui,
    ) {
        // Create text edit
        let response = ui.add(
            TextEdit::singleline(&mut *self.sys.transition_value.borrow_mut())
                .desired_width(100.0)
                .hint_text("value"),
        );
        // Handle focus of text edit
        if response.gained_focus() {
            msgs.push(Message::SetTransitionValueFocused(true));
        }
        if response.lost_focus() {
            msgs.push(Message::SetTransitionValueFocused(false));
        }
        ui.menu_button(
            if self.find_transition_equal {
                "="
            } else {
                "≠"
            },
            |ui| {
                ui.radio(self.find_transition_equal, "=")
                    .clicked()
                    .then(|| {
                        ui.close_menu();
                        msgs.push(Message::SetFindTransitionEqual(true));
                    });
                ui.radio(!self.find_transition_equal, "≠")
                    .clicked()
                    .then(|| {
                        ui.close_menu();
                        msgs.push(Message::SetFindTransitionEqual(false));
                    });
            },
        );
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
        if let Ok(val) = BigUint::from_str_radix((*self.sys.transition_value.borrow()).as_str(), 10)
        {
            msgs.push(Message::MoveCursorToTransition {
                next,
                variable: None,
                transition_type: if self.find_transition_equal {
                    TransitionType::EqualTo(val)
                } else {
                    TransitionType::NotEqualTo(val)
                },
            })
        }
    }
}
