use eyre::Result;
use surfer_translation_types::{
    TranslationPreference, TranslationResult, Translator, ValueKind, ValueRepr, VariableInfo,
    VariableType, VariableValue,
};

use crate::{
    message::Message,
    wave_container::VariableMeta,
    wave_container::{ScopeId, VarId},
};

/// Translator for VCD event signals.
///
/// Event signals in VCD represent discrete occurrences at specific time points.
/// This translator converts event values to "EVENT" strings when triggered,
/// and empty strings otherwise, for clear visualization as impulses.
pub struct EventTranslator {}

impl EventTranslator {
    /// Creates a new event translator.
    pub fn new() -> Self {
        Self {}
    }
}

impl Translator<VarId, ScopeId, Message> for EventTranslator {
    fn name(&self) -> String {
        "Event".to_string()
    }

    fn translate(
        &self,
        _variable: &VariableMeta,
        value: &VariableValue,
    ) -> Result<TranslationResult> {
        // Events are discrete occurrences - each value represents an event trigger
        match value {
            VariableValue::String(s) => {
                // For VCD events, any change to the signal represents an event occurrence
                // The value "1" indicates the event has occurred at this timestamp
                Ok(TranslationResult {
                    val: ValueRepr::String(if s == "1" { "EVENT" } else { "" }.to_string()),
                    kind: ValueKind::Normal,
                    subfields: vec![],
                })
            }
            VariableValue::BigUint(b) => {
                // Events as BigUint - non-zero indicates occurrence
                let event_occurred = *b != 0u32.into();
                Ok(TranslationResult {
                    val: ValueRepr::String(if event_occurred { "EVENT" } else { "" }.to_string()),
                    kind: ValueKind::Normal,
                    subfields: vec![],
                })
            }
        }
    }

    fn variable_info(&self, _variable: &VariableMeta) -> Result<VariableInfo> {
        Ok(VariableInfo::Event)
    }

    fn translates(&self, variable: &VariableMeta) -> Result<TranslationPreference> {
        // Only handle VCD event types
        if variable.variable_type == Some(VariableType::VCDEvent) {
            Ok(TranslationPreference::Prefer)
        } else {
            Ok(TranslationPreference::No)
        }
    }
}
