use num::{bigint::ToBigInt, BigInt, BigUint};
use serde::Deserialize;
use surfer_translation_types::VariableValue;

use crate::{
    displayed_item::{DisplayedItem, DisplayedItemIndex},
    wave_data::WaveData,
};

#[derive(Debug, Deserialize, PartialEq)]
pub enum TransitionType {
    Any,
    FromZero,
    ToZero,
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
                        self.cursor = Some(find_transition_time(
                            next,
                            transition_type,
                            waves,
                            variable,
                            cursor,
                            num_timestamps,
                        ));
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
) -> BigInt {
    let mut new_cursor = cursor.clone();
    if let Ok(Some(res)) = waves.query_variable(
        &variable.variable_ref,
        &cursor.to_biguint().unwrap_or_default(),
    ) {
        if next {
            if let Some(ref time) = res.next {
                let stime = time.to_bigint();
                if stime.is_some() {
                    new_cursor.clone_from(&stime.unwrap());
                }
            } else {
                // No next transition, go to end
                new_cursor.clone_from(num_timestamps);
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
            }
        }

        // if zero edges should be skipped
        if transition_type == TransitionType::FromZero {
            // check if the next transition is 0, if so and requested, go to
            // next positive transition
            let next_value = waves.query_variable(
                &variable.variable_ref,
                &new_cursor.to_biguint().unwrap_or_default(),
            );
            if next_value.is_ok_and(|r| {
                r.is_some_and(|r| {
                    r.current.is_some_and(|v| match v.1 {
                        VariableValue::BigUint(v) => v == BigUint::from(0u8),
                        _ => false,
                    })
                })
            }) {
                new_cursor = find_transition_time(
                    next,
                    TransitionType::Any,
                    waves,
                    variable,
                    &new_cursor,
                    num_timestamps,
                );
            }
        }
    }
    new_cursor
}
