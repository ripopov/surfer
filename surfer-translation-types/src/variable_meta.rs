#[cfg(feature = "wasm_plugins")]
use extism_convert::{FromBytes, Json, ToBytes};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{VariableDirection, VariableEncoding, VariableIndex, VariableRef, VariableType};

#[cfg_attr(feature = "wasm_plugins", derive(FromBytes, ToBytes))]
#[cfg_attr(feature = "wasm_plugins", encoding(Json))]
#[derive(Clone, Debug, Serialize, Deserialize)]
/// Additional information about a variable in the waveform.
pub struct VariableMeta<VarId, ScopeId> {
    /// Reference to the variable.
    pub var: VariableRef<VarId, ScopeId>,
    /// Number of bits for the variable, if applicable.
    pub num_bits: Option<u32>,
    /// Type of the variable in the HDL (on a best effort basis).
    pub variable_type: Option<VariableType>,
    /// Type name of variable, if available.
    pub variable_type_name: Option<String>,
    /// Index information for the variable, if available.
    pub index: Option<VariableIndex>,
    /// Direction of the variable, if available.
    pub direction: Option<VariableDirection>,
    /// For enum variables, either an enumerated type in VHDL or an enum in SystemVerilog,
    /// a mapping from enum option names to their string representations.
    pub enum_map: HashMap<String, String>,
    /// Indicates how the variable is stored. A variable of "type" boolean for example
    /// could be stored as a String or as a `BitVector`.
    pub encoding: VariableEncoding,
}

impl<VarId, ScopeId> VariableMeta<VarId, ScopeId> {
    /// Parameter
    pub fn is_parameter(&self) -> bool {
        matches!(
            self.variable_type,
            Some(VariableType::VCDParameter | VariableType::RealParameter)
        )
    }

    /// Parameter
    pub fn is_event(&self) -> bool {
        matches!(self.variable_type, Some(VariableType::VCDEvent))
    }

    /// Types that should default to signed integer conversion
    pub fn is_integer_type(&self) -> bool {
        matches!(
            self.variable_type,
            Some(
                VariableType::VCDInteger
                    | VariableType::Int
                    | VariableType::ShortInt
                    | VariableType::LongInt
            )
        )
    }

    /// Check if the variable type name indicates signed integer type
    pub fn has_signed_integer_type_name(&self) -> bool {
        match_variable_type_name(&self.variable_type_name, SIGNED_INTEGER_TYPE_NAMES)
    }

    /// Check if the variable type name indicates signed fixed-point type
    pub fn has_signed_fixedpoint_type_name(&self) -> bool {
        match_variable_type_name(&self.variable_type_name, SIGNED_FIXEDPOINT_TYPE_NAMES)
    }

    /// Check if the variable type name indicates unsigned integer type
    pub fn has_unsigned_integer_type_name(&self) -> bool {
        match_variable_type_name(&self.variable_type_name, UNSIGNED_INTEGER_TYPE_NAMES)
    }

    /// Check if the variable type name indicates unsigned fixed-point type
    pub fn has_unsigned_fixedpoint_type_name(&self) -> bool {
        match_variable_type_name(&self.variable_type_name, UNSIGNED_FIXEDPOINT_TYPE_NAMES)
    }
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

/// Helper to case insensitive match of variable type names against a list of candidates
fn match_variable_type_name(
    variable_type_name: &Option<String>,
    candidates: &'static [&'static str],
) -> bool {
    variable_type_name
        .as_ref()
        .is_some_and(|type_name| candidates.iter().any(|c| type_name.eq_ignore_ascii_case(c)))
}

/// Type names that should default to signed integer conversion
/// - `ieee.numeric_std.signed`
static SIGNED_INTEGER_TYPE_NAMES: &[&str] = &["unresolved_signed", "signed"];

/// Type names that should default to signed fixed-point conversion
/// - `ieee.fixed_pkg.sfixed`
static SIGNED_FIXEDPOINT_TYPE_NAMES: &[&str] = &["unresolved_sfixed", "sfixed"];

/// Type names that should default to unsigned integer conversion
/// - `ieee.numeric_std.unsigned`
static UNSIGNED_INTEGER_TYPE_NAMES: &[&str] = &["unresolved_unsigned", "unsigned"];

/// Type names that should default to unsigned fixed-point conversion
/// - `ieee.fixed_pkg.ufixed`
static UNSIGNED_FIXEDPOINT_TYPE_NAMES: &[&str] = &["unresolved_ufixed", "ufixed"];
