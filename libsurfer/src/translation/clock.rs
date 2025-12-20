use std::sync::Arc;
use surfer_translation_types::{TranslationResult, Translator, VariableInfo, VariableValue};

use crate::message::Message;
use crate::translation::{AnyTranslator, BitTranslator};
use crate::wave_container::{ScopeId, VarId, VariableMeta};

pub struct ClockTranslator {
    // In order to not duplicate logic, we reuse the bit translator internally
    inner: AnyTranslator,
}

impl ClockTranslator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: AnyTranslator::Basic(Arc::new(BitTranslator {})),
        }
    }
}

impl Default for ClockTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator<VarId, ScopeId, Message> for ClockTranslator {
    fn name(&self) -> String {
        "Clock".to_string()
    }

    fn translate(
        &self,
        variable: &VariableMeta,
        value: &VariableValue,
    ) -> eyre::Result<TranslationResult> {
        self.inner.translate(variable, value)
    }

    fn variable_info(&self, _variable: &VariableMeta) -> eyre::Result<VariableInfo> {
        Ok(VariableInfo::Clock)
    }

    fn translates(&self, variable: &VariableMeta) -> eyre::Result<super::TranslationPreference> {
        if variable.num_bits == Some(1) {
            Ok(super::TranslationPreference::Yes)
        } else {
            Ok(super::TranslationPreference::No)
        }
    }
}
