use crate::{ValueKind, VariableMeta, VariableValue};
use extism_convert::{FromBytes, Json, ToBytes};
use serde::{Deserialize, Serialize};

#[derive(FromBytes, ToBytes, Deserialize, Serialize)]
#[encoding(Json)]
pub struct TranslateParams {
    pub variable: VariableMeta<(), ()>,
    pub value: VariableValue,
}

#[derive(FromBytes, ToBytes, Deserialize, Serialize)]
#[encoding(Json)]
pub struct BasicTranslateParams {
    pub num_bits: u64,
    pub value: VariableValue,
}

#[derive(FromBytes, ToBytes, Deserialize, Serialize)]
#[encoding(Json)]
pub struct BasicTranslateResult {
    pub value: String,
    pub kind: ValueKind,
}
