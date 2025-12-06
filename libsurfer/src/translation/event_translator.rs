use surfer_translation_types::{
    TranslationResult, Translator, ValueKind, VariableInfo, VariableType, VariableValue,
};

use crate::message::Message;
use crate::wave_container::{ScopeId, VarId, VariableMeta};

pub struct EventTranslator;

impl Translator<VarId, ScopeId, Message> for EventTranslator {
    fn name(&self) -> String {
        "Event".to_string()
    }

    fn translate(
        &self,
        _variable: &VariableMeta,
        _value: &VariableValue,
    ) -> eyre::Result<TranslationResult> {
        Ok(TranslationResult {
            val: surfer_translation_types::ValueRepr::Event,
            subfields: vec![],
            kind: ValueKind::Event,
        })
    }

    fn variable_info(&self, _variable: &VariableMeta) -> eyre::Result<VariableInfo> {
        Ok(VariableInfo::Event)
    }

    fn translates(&self, variable: &VariableMeta) -> eyre::Result<super::TranslationPreference> {
        if variable.num_bits == Some(0) {
            match &variable.variable_type {
                Some(VariableType::VCDEvent) => Ok(super::TranslationPreference::Prefer),
                _ => Ok(super::TranslationPreference::No),
            }
        } else {
            Ok(super::TranslationPreference::No)
        }
    }
}
