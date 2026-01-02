use crate::wave_container::VariableMeta;
#[local_impl::local_impl]
impl VariableMetaExt for VariableMeta {
    /// Check if the variable type name indicates signed integer type
    fn has_signed_integer_type_name(&self) -> bool {
        match_variable_type_name(self.variable_type_name.as_ref(), SIGNED_INTEGER_TYPE_NAMES)
    }

    /// Check if the variable type name indicates signed fixed-point type
    fn has_signed_fixedpoint_type_name(&self) -> bool {
        match_variable_type_name(
            self.variable_type_name.as_ref(),
            SIGNED_FIXEDPOINT_TYPE_NAMES,
        )
    }

    /// Check if the variable type name indicates unsigned integer type
    fn has_unsigned_integer_type_name(&self) -> bool {
        match_variable_type_name(
            self.variable_type_name.as_ref(),
            UNSIGNED_INTEGER_TYPE_NAMES,
        )
    }

    /// Check if the variable type name indicates unsigned fixed-point type
    fn has_unsigned_fixedpoint_type_name(&self) -> bool {
        match_variable_type_name(
            self.variable_type_name.as_ref(),
            UNSIGNED_FIXEDPOINT_TYPE_NAMES,
        )
    }
}

/// Helper to case insensitive match of variable type names against a list of candidates
fn match_variable_type_name(
    variable_type_name: Option<&String>,
    candidates: &'static [&'static str],
) -> bool {
    variable_type_name
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
