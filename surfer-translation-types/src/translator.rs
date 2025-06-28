//! Definition of the main [`Translator`] trait and the simplified version
//! [`BasicTranslator`].
use extism_convert::{FromBytes, Json, ToBytes};
use eyre::Result;
use num::BigUint;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;

use crate::result::TranslationResult;
use crate::{
    TranslationPreference, ValueKind, VariableEncoding, VariableInfo, VariableMeta, VariableValue,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromBytes, ToBytes)]
#[encoding(Json)]
pub enum TrueName {
    /// The variable's true name is best represented as part of a line of code
    /// for example if line 100 is
    /// let x = a + b;
    /// and the signal being queried is `a+b` then this would return
    /// {line: 100, before: "let x = ", this: "a + b", after: ";"}
    SourceCode {
        line_number: usize,
        before: String,
        this: String,
        after: String,
    },
}

/// Provides a way for translators to "change" the name of variables in the variable list.
/// Most translators should not produce VariableNameInfo since it is a global thing that
/// is done on _all_ variables, not just those which have had the translator applied.
///
/// An example use case is translators for HDLs which want to translate from automatically
/// generated subexpression back into names that a human can understand. In this use case,
/// it is _very_ unlikely that the user wants to see the raw anonymous name that the compiler
/// emitted, so performing this translation globally makes sense.
#[derive(Clone, Debug, Serialize, Deserialize, FromBytes, ToBytes)]
#[encoding(Json)]
pub struct VariableNameInfo {
    /// A more human-undesrstandable name for a signal. This should only be used by translators
    /// which
    pub true_name: Option<TrueName>,
    /// Translators can change the order that signals appear in the variable list using this
    /// parameter. Before rendering, the variable will be sported by this number in descending
    /// order, so variables that are predicted to be extra important to the
    /// user should have a number > 0 while unimportant variables should be < 0
    ///
    /// Translators should only poke at this variable if they know something about the variable.
    /// For example, an HDL translator that does not recognise a name should leave it at None
    /// to give other translators the chance to set the priority
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromBytes, ToBytes)]
#[encoding(Json)]
pub enum WaveSource {
    File(String),
    Data,
    DragAndDrop(Option<String>),
    Url(String),
    Cxxrtl,
}

/// The most general translator trait.
pub trait Translator<VarId, ScopeId, Message>: Send + Sync {
    fn name(&self) -> String;

    /// Notify the translator that the wave source has changed to the specified source
    fn set_wave_source(&self, _wave_source: Option<WaveSource>) {}

    fn translate(
        &self,
        variable: &VariableMeta<VarId, ScopeId>,
        value: &VariableValue,
    ) -> Result<TranslationResult>;

    fn variable_info(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<VariableInfo>;

    /// Return [`TranslationPreference`] based on if the translator can handle this variable.
    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference>;

    /// By default translators are stateless, but if they need to reload, they can
    /// do by defining this method.
    /// Long running translators should run the reloading in the background using `perform_work`
    fn reload(&self, _sender: Sender<Message>) {}

    /// Returns a `VariableNameInfo` about the specified variable which will be applied globally.
    /// Most translators should simply return `None` here, see the
    /// documentation `VariableNameInfo` for exceptions to this rule.
    fn variable_name_info(
        &self,
        variable: &VariableMeta<VarId, ScopeId>,
    ) -> Option<VariableNameInfo> {
        // We could name this `_variable`, but that means the docs will make it look unused
        // and LSPs will fill in the definition with that name too, so we'll mark it as unused
        // like this
        let _ = variable;
        None
    }
}

/// A translator that only produces non-hierarchical values
pub trait BasicTranslator<VarId, ScopeId>: Send + Sync {
    fn name(&self) -> String;

    fn basic_translate(&self, num_bits: u64, value: &VariableValue) -> (String, ValueKind);

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        translates_all_bit_types(variable)
    }

    fn variable_info(&self, _variable: &VariableMeta<VarId, ScopeId>) -> Result<VariableInfo> {
        Ok(VariableInfo::Bits)
    }
}

enum NumberParseResult {
    Numerical(BigUint),
    Unparsable(String, ValueKind),
}

/// Turn vector variable string into name and corresponding kind if it
/// includes values other than 0 and 1. If only 0 and 1, return None.
fn map_vector_variable(s: &str) -> NumberParseResult {
    if let Some(val) = BigUint::parse_bytes(s.as_bytes(), 2) {
        NumberParseResult::Numerical(val)
    } else if s.contains('x') {
        NumberParseResult::Unparsable("UNDEF".to_string(), ValueKind::Undef)
    } else if s.contains('z') {
        NumberParseResult::Unparsable("HIGHIMP".to_string(), ValueKind::HighImp)
    } else if s.contains('-') {
        NumberParseResult::Unparsable("DON'T CARE".to_string(), ValueKind::DontCare)
    } else if s.contains('u') {
        NumberParseResult::Unparsable("UNDEF".to_string(), ValueKind::Undef)
    } else if s.contains('w') {
        NumberParseResult::Unparsable("UNDEF WEAK".to_string(), ValueKind::Undef)
    } else if s.contains('h') || s.contains('l') {
        NumberParseResult::Unparsable("WEAK".to_string(), ValueKind::Weak)
    } else {
        NumberParseResult::Unparsable("UNKNOWN VALUES".to_string(), ValueKind::Undef)
    }
}

impl VariableValue {
    pub fn parse_biguint(self) -> Result<BigUint, (String, ValueKind)> {
        match self {
            VariableValue::BigUint(v) => Ok(v),
            VariableValue::String(s) => match map_vector_variable(&s) {
                NumberParseResult::Unparsable(v, k) => Err((v, k)),
                NumberParseResult::Numerical(v) => Ok(v),
            },
        }
    }
}

pub fn translates_all_bit_types<VarId, ScopeId>(
    variable: &VariableMeta<VarId, ScopeId>,
) -> Result<TranslationPreference> {
    if variable.encoding == VariableEncoding::BitVector {
        Ok(TranslationPreference::Yes)
    } else {
        Ok(TranslationPreference::No)
    }
}
