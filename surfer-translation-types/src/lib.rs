mod field_ref;
pub mod plugin_types;
#[cfg(feature = "pyo3")]
pub mod python;
mod result;
mod scope_ref;
pub mod translator;
pub mod variable_index;
mod variable_ref;

use derive_more::Display;
use ecolor::Color32;
#[cfg(feature = "wasm_plugins")]
use extism_convert::{FromBytes, Json, ToBytes};
use num::BigUint;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use crate::field_ref::FieldRef;
pub use crate::result::{
    HierFormatResult, SubFieldFlatTranslationResult, SubFieldTranslationResult, TranslatedValue,
    TranslationResult, ValueRepr,
};
pub use crate::scope_ref::ScopeRef;
pub use crate::translator::{
    BasicTranslator, Translator, VariableNameInfo, WaveSource, translates_all_bit_types,
};
pub use crate::variable_index::VariableIndex;
pub use crate::variable_ref::VariableRef;

#[cfg_attr(feature = "wasm_plugins", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "wasm_plugins", encoding(Json))]
#[derive(Deserialize, Serialize)]
pub struct PluginConfig(pub HashMap<String, String>);

/// Turn vector variable string into name and corresponding color if it
/// includes values other than 0 and 1. If only 0 and 1, return None.
pub fn check_vector_variable(s: &str) -> Option<(String, ValueKind)> {
    if s.contains('x') {
        Some(("UNDEF".to_string(), ValueKind::Undef))
    } else if s.contains('z') {
        Some(("HIGHIMP".to_string(), ValueKind::HighImp))
    } else if s.contains('-') {
        Some(("DON'T CARE".to_string(), ValueKind::DontCare))
    } else if s.contains('u') {
        Some(("UNDEF".to_string(), ValueKind::Undef))
    } else if s.contains('w') {
        Some(("UNDEF WEAK".to_string(), ValueKind::Undef))
    } else if s.contains('h') || s.contains('l') {
        Some(("WEAK".to_string(), ValueKind::Weak))
    } else if s.chars().all(|c| matches!(c, '0' | '1')) {
        None
    } else {
        Some(("UNKNOWN VALUES".to_string(), ValueKind::Undef))
    }
}

/// Return kind for a binary representation
pub fn kind_for_binary_representation(s: &str) -> ValueKind {
    if s.contains('x') {
        ValueKind::Undef
    } else if s.contains('z') {
        ValueKind::HighImp
    } else if s.contains('-') {
        ValueKind::DontCare
    } else if s.contains('u') || s.contains('w') {
        ValueKind::Undef
    } else if s.contains('h') || s.contains('l') {
        ValueKind::Weak
    } else {
        ValueKind::Normal
    }
}

/// VCD bit extension
pub fn extend_string(val: &str, num_bits: u64) -> String {
    if num_bits > val.len() as u64 {
        let extra_count = num_bits - val.len() as u64;
        let extra_value = match val.chars().next() {
            Some('0') => "0",
            Some('1') => "0",
            Some('x') => "x",
            Some('z') => "z",
            // If we got weird characters, this is probably a string, so we don't
            // do the extension
            // We may have to add extensions for std_logic values though if simulators save without extension
            _ => "",
        };
        extra_value.repeat(extra_count as usize)
    } else {
        String::new()
    }
}

#[derive(Debug, PartialEq, Clone, Display, Serialize, Deserialize)]
pub enum VariableValue {
    #[display("{_0}")]
    BigUint(BigUint),
    #[display("{_0}")]
    String(String),
}

impl VariableValue {
    /// Utility function for handling the happy case of the variable value being only 0 and 1,
    /// with default handling of other values.
    ///
    /// The value passed to the handler is guaranteed to only contain 0 and 1, but it is not
    /// padded to the length of the vector, i.e. leading zeros can be missing. Use [extend_string]
    /// on the result to add the padding.
    pub fn handle_bits<E>(
        self,
        handler: impl Fn(String) -> Result<TranslationResult, E>,
    ) -> Result<TranslationResult, E> {
        let value = match self {
            VariableValue::BigUint(v) => format!("{v:b}"),
            VariableValue::String(v) => {
                if let Some((val, kind)) = check_vector_variable(&v) {
                    return Ok(TranslationResult {
                        val: ValueRepr::String(val),
                        subfields: vec![],
                        kind,
                    });
                } else {
                    v
                }
            }
        };

        handler(value)
    }
}

#[derive(Clone, PartialEq, Copy, Debug, Serialize, Deserialize)]
pub enum ValueKind {
    Normal,
    Undef,
    HighImp,
    Custom(Color32),
    Warn,
    DontCare,
    Weak,
    Error,
}

#[cfg_attr(feature = "wasm_plugins", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "wasm_plugins", encoding(Json))]
#[derive(PartialEq, Deserialize, Serialize, Debug)]
pub enum TranslationPreference {
    /// This translator prefers translating the variable, so it will be selected
    /// as the default translator for the variable
    Prefer,
    /// This translator is able to translate the variable, but will not be
    /// selected by default, the user has to select it
    Yes,
    No,
}

/// Static information about the structure of a variable.
#[cfg_attr(feature = "wasm_plugins", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "wasm_plugins", encoding(Json))]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub enum VariableInfo {
    Compound {
        subfields: Vec<(String, VariableInfo)>,
    },
    Bits,
    Bool,
    Clock,
    // NOTE: only used for state saving where translators will clear this out with the actual value
    #[default]
    String,
    Real,
}

#[derive(Debug, Display, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum VariableType {
    // VCD-specific types
    #[display("event")]
    VCDEvent,
    #[display("reg")]
    VCDReg,
    #[display("wire")]
    VCDWire,
    #[display("real")]
    VCDReal,
    #[display("time")]
    VCDTime,
    #[display("string")]
    VCDString,
    #[display("parameter")]
    VCDParameter,
    #[display("integer")]
    VCDInteger,
    #[display("real time")]
    VCDRealTime,
    #[display("supply 0")]
    VCDSupply0,
    #[display("supply 1")]
    VCDSupply1,
    #[display("tri")]
    VCDTri,
    #[display("tri and")]
    VCDTriAnd,
    #[display("tri or")]
    VCDTriOr,
    #[display("tri reg")]
    VCDTriReg,
    #[display("tri 0")]
    VCDTri0,
    #[display("tri 1")]
    VCDTri1,
    #[display("wand")]
    VCDWAnd,
    #[display("wor")]
    VCDWOr,
    #[display("port")]
    Port,
    #[display("sparse array")]
    SparseArray,
    #[display("realtime")]
    RealTime,

    // System Verilog
    #[display("bit")]
    Bit,
    #[display("logic")]
    Logic,
    #[display("int")]
    Int,
    #[display("shortint")]
    ShortInt,
    #[display("longint")]
    LongInt,
    #[display("byte")]
    Byte,
    #[display("enum")]
    Enum,
    #[display("shortreal")]
    ShortReal,

    // VHDL (these are the types emitted by GHDL)
    #[display("boolean")]
    Boolean,
    #[display("bit_vector")]
    BitVector,
    #[display("std_logic")]
    StdLogic,
    #[display("std_logic_vector")]
    StdLogicVector,
    #[display("std_ulogic")]
    StdULogic,
    #[display("std_ulogic_vector")]
    StdULogicVector,
}

#[derive(Clone, Display, Copy, PartialOrd, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VariableDirection {
    // Ordering is used for sorting variable list
    #[display("input")]
    Input,
    #[display("output")]
    Output,
    #[display("inout")]
    InOut,
    #[display("buffer")]
    Buffer,
    #[display("linkage")]
    Linkage,
    #[display("implicit")]
    Implicit,
    #[display("unknown")]
    Unknown,
}

#[cfg_attr(feature = "wasm_plugins", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "wasm_plugins", encoding(Json))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VariableMeta<VarId, ScopeId> {
    pub var: VariableRef<VarId, ScopeId>,
    pub num_bits: Option<u32>,
    /// Type of the variable in the HDL (on a best effort basis).
    pub variable_type: Option<VariableType>,
    /// Type name of variable, if available
    pub variable_type_name: Option<String>,
    pub index: Option<VariableIndex>,
    pub direction: Option<VariableDirection>,
    pub enum_map: HashMap<String, String>,
    /// Indicates how the variable is stored. A variable of "type" boolean for example
    /// could be stored as a String or as a BitVector.
    pub encoding: VariableEncoding,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum VariableEncoding {
    String,
    Real,
    BitVector,
}

impl<VarId1, ScopeId1> VariableMeta<VarId1, ScopeId1> {
    pub fn map_ids<VarId2, ScopeId2>(
        self,
        var_fn: impl FnMut(VarId1) -> VarId2,
        scope_fn: impl FnMut(ScopeId1) -> ScopeId2,
    ) -> VariableMeta<VarId2, ScopeId2> {
        VariableMeta {
            var: self.var.map_ids(var_fn, scope_fn),
            num_bits: self.num_bits,
            variable_type: self.variable_type,
            index: self.index,
            direction: self.direction,
            enum_map: self.enum_map,
            encoding: self.encoding,
            variable_type_name: self.variable_type_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ValueKind, check_vector_variable, extend_string};

    #[test]
    fn binary_only_returns_none() {
        for s in ["0", "1", "0101", "1111", "000000", "101010"].iter() {
            assert_eq!(check_vector_variable(s), None, "{s}");
        }
    }

    #[test]
    fn x_marks_undef() {
        let res = check_vector_variable("10x01").unwrap();
        assert_eq!(res.0, "UNDEF");
        assert_eq!(res.1, ValueKind::Undef);
    }

    #[test]
    fn u_marks_undef() {
        for s in ["u", "10u", "uuuu"].iter() {
            let res = check_vector_variable(s).unwrap();
            assert_eq!(res.0, "UNDEF");
            assert_eq!(res.1, ValueKind::Undef);
        }
    }

    #[test]
    fn z_marks_highimp() {
        let res = check_vector_variable("zz01").unwrap();
        assert_eq!(res.0, "HIGHIMP");
        assert_eq!(res.1, ValueKind::HighImp);
    }

    #[test]
    fn dash_marks_dont_care() {
        let res = check_vector_variable("-01--").unwrap();
        assert_eq!(res.0, "DON'T CARE");
        assert_eq!(res.1, ValueKind::DontCare);
    }

    #[test]
    fn w_marks_undef_weak() {
        let res = check_vector_variable("w101").unwrap();
        assert_eq!(res.0, "UNDEF WEAK");
        assert_eq!(res.1, ValueKind::Undef); // intentionally Undef per implementation
    }

    #[test]
    fn h_or_l_marks_weak() {
        let res_h = check_vector_variable("h110").unwrap();
        assert_eq!(res_h.0, "WEAK");
        assert_eq!(res_h.1, ValueKind::Weak);

        let res_l = check_vector_variable("l001").unwrap();
        assert_eq!(res_l.0, "WEAK");
        assert_eq!(res_l.1, ValueKind::Weak);
    }

    #[test]
    fn unknown_values_fallback() {
        for s in ["2", "a", "?", " "] {
            let res = check_vector_variable(s).unwrap();
            assert_eq!(res.0, "UNKNOWN VALUES");
            assert_eq!(res.1, ValueKind::Undef);
        }
    }

    #[test]
    fn precedence_is_respected() {
        // contains both x and z -> x handled first (UNDEF)
        let res = check_vector_variable("xz").unwrap();
        assert_eq!(res.0, "UNDEF");
        assert_eq!(res.1, ValueKind::Undef);

        // contains w and h -> w handled before h, yielding UNDEF WEAK not WEAK
        let res = check_vector_variable("wh").unwrap();
        assert_eq!(res.0, "UNDEF WEAK");
        assert_eq!(res.1, ValueKind::Undef);
    }

    // ---------------- extend_string tests ----------------

    #[test]
    fn extend_string_zero_extend_from_0_and_1() {
        // Leading '0' extends with '0'
        assert_eq!(extend_string("001", 5), "00");
        assert_eq!(extend_string("0", 3), "00");

        // Leading '1' also extends with '0' per current implementation
        assert_eq!(extend_string("101", 5), "00");
        assert_eq!(extend_string("1", 4), "000");
    }

    #[test]
    fn extend_string_x_and_z() {
        // Leading 'x' extends with 'x'
        assert_eq!(extend_string("x1", 4), "xx");
        assert_eq!(extend_string("x", 3), "xx");

        // Leading 'z' extends with 'z'
        assert_eq!(extend_string("z0", 3), "z");
        assert_eq!(extend_string("z", 5), "zzzz");
    }

    #[test]
    fn extend_string_same_or_smaller_returns_empty() {
        assert_eq!(extend_string("101", 3), "");
        assert_eq!(extend_string("101", 2), "");
        assert_eq!(extend_string("", 0), "");
    }

    #[test]
    fn extend_string_weird_char_and_empty_input() {
        // Unknown leading char results in no extension (empty), even if num_bits is larger
        assert_eq!(extend_string("h101", 6), "");
        assert_eq!(extend_string("?", 10), "");

        // Empty input yields empty extension as there is no leading char to guide
        assert_eq!(extend_string("", 5), "");
    }
}
