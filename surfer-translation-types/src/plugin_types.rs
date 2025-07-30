use crate::{VariableMeta, VariableValue};
#[cfg(feature = "wasm_plugins")]
use extism_convert::{FromBytes, Json, ToBytes};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "wasm_plugins", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "wasm_plugins", encoding(Json))]
#[derive(Deserialize, Serialize)]
pub struct TranslateParams {
    pub variable: VariableMeta<(), ()>,
    pub value: VariableValue,
}
