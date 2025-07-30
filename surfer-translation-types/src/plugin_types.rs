use crate::{VariableMeta, VariableValue};
#[cfg(feature = "extism-convert")]
use extism_convert::{FromBytes, Json, ToBytes};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "extism-convert", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "extism-convert", encoding(Json))]
#[derive(Deserialize, Serialize)]
pub struct TranslateParams {
    pub variable: VariableMeta<(), ()>,
    pub value: VariableValue,
}
