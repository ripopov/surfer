use crate::message::Message;
use crate::translation::{TranslationPreference, ValueKind, VariableInfo};
use crate::wave_container::{ScopeId, VarId, VariableMeta};
use color_eyre::Result;
use std::borrow::Cow;
use surfer_translation_types::{
    TranslationResult, Translator, TranslatorInfo, ValueRepr, VariableValue,
};

pub struct EnumTranslator {
    variable: VariableMeta,
}

impl Translator<VarId, ScopeId, Message> for EnumTranslator {
    fn translate(&self, value: &VariableValue) -> Result<TranslationResult> {
        let str_value = match value {
            VariableValue::BigUint(v) => Cow::Owned(format!(
                "{v:0width$b}",
                width = self.variable.num_bits.unwrap() as usize
            )),
            VariableValue::String(s) => Cow::Borrowed(s),
        };
        let (kind, name) = self
            .variable
            .enum_map
            .get(str_value.as_str())
            .map(|s| (ValueKind::Normal, s.to_string()))
            .unwrap_or((ValueKind::Warn, format!("ERROR ({str_value})")));
        Ok(TranslationResult {
            val: ValueRepr::String(name),
            kind,
            subfields: vec![],
        })
    }

    fn variable_info(&self) -> color_eyre::Result<VariableInfo> {
        Ok(VariableInfo::Bits)
    }
}

pub struct EnumTranslatorInfo {}

impl TranslatorInfo<VarId, ScopeId, Message> for EnumTranslatorInfo {
    fn name(&self) -> String {
        "Enum".to_string()
    }

    fn translates(&self, variable: &VariableMeta) -> Result<TranslationPreference> {
        if variable.enum_map.is_empty() {
            Ok(TranslationPreference::No)
        } else {
            Ok(TranslationPreference::Prefer)
        }
    }

    type Translator = EnumTranslator;

    fn create_instance(
        &self,
        variable: &surfer_translation_types::VariableMeta<VarId, ScopeId>,
    ) -> Self::Translator {
        EnumTranslator {
            variable: variable.clone(),
        }
    }
}
