use crate::variable_type::INTEGER_TYPES;
use crate::wave_container::{ScopeId, VarId};
use color_eyre::Result;
use half::{bf16, f16};
use num::ToPrimitive;
use num::{BigInt, BigUint};

use softposit::{P16E1, P32E2, P8E0, Q16E1, Q8E0};
use surfer_translation_types::{
    translates_all_bit_types, NumericTranslator, NumericalValueRepr, VariableMeta,
};

use super::{check_single_wordlength, match_variable_type_name, TranslationPreference};

fn biguint_signed(v: &BigUint, num_bits: u32) -> BigInt {
    let signweight = BigUint::from(1u8) << (num_bits - 1);
    if v < &signweight {
        BigInt::from(v.clone())
    } else {
        let v2 = (signweight << 1) - v;
        -BigInt::from(v2)
    }
}

pub struct UnsignedTranslator {}

impl NumericTranslator<VarId, ScopeId> for UnsignedTranslator {
    fn name(&self) -> String {
        String::from("Unsigned")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        Some(NumericalValueRepr::Integer(v.clone().into()))
    }

    fn variable_range(&self, meta: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        let max: BigInt = (BigInt::from(1u64) << meta.num_bits.unwrap()) - 1;
        (0.0, max.to_f64().unwrap())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        let candidates = ["unresolved_unsigned".to_string(), "unsigned".to_string()];
        if match_variable_type_name(&variable.variable_type_name, &candidates) {
            Ok(TranslationPreference::Prefer)
        } else {
            translates_all_bit_types(variable)
        }
    }
}

pub struct SignedTranslator {}

impl NumericTranslator<VarId, ScopeId> for SignedTranslator {
    fn name(&self) -> String {
        String::from("Signed")
    }

    fn numeric_translate(
        &self,
        meta: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        Some(NumericalValueRepr::Integer(biguint_signed(
            v,
            meta.num_bits.unwrap(),
        )))
    }

    fn variable_range(&self, meta: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        let max: BigInt = (BigInt::from(1u64) << (meta.num_bits.unwrap() - 1)) - 1;
        let min: BigInt = max.clone() * -1 + 1;
        (min.to_f64().unwrap(), max.to_f64().unwrap())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        let candidates = ["unresolved_signed".to_string(), "signed".to_string()];
        if INTEGER_TYPES.contains(&variable.variable_type)
            | match_variable_type_name(&variable.variable_type_name, &candidates)
        {
            Ok(TranslationPreference::Prefer)
        } else {
            translates_all_bit_types(variable)
        }
    }
}

pub struct SinglePrecisionTranslator {}

impl NumericTranslator<VarId, ScopeId> for SinglePrecisionTranslator {
    fn name(&self) -> String {
        String::from("FP: 32-bit IEEE 754")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u32()
            .map(f32::from_bits)
            .map(f64::from)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (f32::MIN.into(), f32::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 32)
    }
}

pub struct DoublePrecisionTranslator {}

impl NumericTranslator<VarId, ScopeId> for DoublePrecisionTranslator {
    fn name(&self) -> String {
        String::from("FP: 64-bit IEEE 754")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u64()
            .map(f64::from_bits)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (f64::MIN.into(), f64::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 64)
    }
}

#[cfg(feature = "f128")]
pub struct QuadPrecisionTranslator {}

#[cfg(feature = "f128")]
impl NumericTranslator<VarId, ScopeId> for QuadPrecisionTranslator {
    fn name(&self) -> String {
        String::from("FP: 128-bit IEEE 754")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        // TODO: we lose precision here
        v.to_u128()
            .map(f128::from_bits)
            .map(|v| NumericalValueRepr::FloatingPoint(v.to_f64()));
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (f128::MIN.into(), f128::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 128)
    }
}

pub struct HalfPrecisionTranslator {}

impl NumericTranslator<VarId, ScopeId> for HalfPrecisionTranslator {
    fn name(&self) -> String {
        String::from("FP: 16-bit IEEE 754")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u16()
            .map(f16::from_bits)
            .map(f64::from)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (f16::MIN.into(), f16::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 16)
    }
}

pub struct BFloat16Translator {}

impl NumericTranslator<VarId, ScopeId> for BFloat16Translator {
    fn name(&self) -> String {
        String::from("FP: bfloat16")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u16()
            .map(bf16::from_bits)
            .map(f64::from)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (bf16::MIN.into(), bf16::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 16)
    }
}

pub struct Posit32Translator {}

impl NumericTranslator<VarId, ScopeId> for Posit32Translator {
    fn name(&self) -> String {
        String::from("Posit: 32-bit (two exponent bits)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u32()
            .map(P32E2::from_bits)
            .map(f64::from)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (P32E2::MIN.into(), P32E2::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 32)
    }
}

pub struct Posit16Translator {}

impl NumericTranslator<VarId, ScopeId> for Posit16Translator {
    fn name(&self) -> String {
        String::from("Posit: 16-bit (one exponent bit)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u16()
            .map(P16E1::from_bits)
            .map(f64::from)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (P16E1::MIN.into(), P16E1::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 16)
    }
}

pub struct Posit8Translator {}

impl NumericTranslator<VarId, ScopeId> for Posit8Translator {
    fn name(&self) -> String {
        String::from("Posit: 8-bit (no exponent bit)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u8()
            .map(P8E0::from_bits)
            .map(f64::from)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (P8E0::MIN.into(), P8E0::MAX.into())
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 8)
    }
}

pub struct PositQuire8Translator {}

impl NumericTranslator<VarId, ScopeId> for PositQuire8Translator {
    fn name(&self) -> String {
        String::from("Posit: quire for 8-bit (no exponent bit)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u32()
            .map(Q8E0::from_bits)
            .map(|v| f64::from(v.to_posit()))
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (0.0, 1.0) // TODO
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 32)
    }
}

pub struct PositQuire16Translator {}

impl NumericTranslator<VarId, ScopeId> for PositQuire16Translator {
    fn name(&self) -> String {
        String::from("Posit: quire for 16-bit (one exponent bit)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u128()
            .map(Q16E1::from_bits)
            .map(|v| f64::from(v.to_posit()))
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (0.0, 1.0) // TODO
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 128)
    }
}

#[allow(clippy::excessive_precision)]
/// Decode u8 as 8-bit float with five exponent bits and two mantissa bits
fn decode_e5m2(v: u8) -> f64 {
    let mant = v & 3;
    let exp = (v >> 2) & 31;
    let sign: i8 = 1 - ((v >> 6) & 2) as i8; // 1 - 2*signbit
    match (exp, mant) {
        (31, 0) => f64::INFINITY,
        (31, ..) => f64::NAN,
        (0, 0) => {
            if sign == -1 {
                -0.0f64
            } else {
                0.0f64
            }
        }
        (0, ..) => ((sign * mant as i8) as f64) * 0.0000152587890625f64, // 0.0000152587890625 = 2^-16
        _ => ((sign * (4 + mant as i8)) as f64) * 2.0f64.powi(exp as i32 - 17), // 17 = 15 (bias) + 2 (mantissa bits)
    }
}

pub struct E5M2Translator {}

impl NumericTranslator<VarId, ScopeId> for E5M2Translator {
    fn name(&self) -> String {
        String::from("FP: 8-bit (E5M2)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u8()
            .map(decode_e5m2)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (0.0, 1.0) // TODO
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 8)
    }
}

/// Decode u8 as 8-bit float with four exponent bits and three mantissa bits
fn decode_e4m3(v: u8) -> f64 {
    let mant = v & 7;
    let exp = (v >> 3) & 15;
    let sign: i8 = 1 - ((v >> 6) & 2) as i8; // 1 - 2*signbit
    match (exp, mant) {
        (15, 7) => f64::NAN,
        (0, 0) => {
            if sign == -1 {
                -0.0f64
            } else {
                0.0f64
            }
        }
        (0, ..) => ((sign * mant as i8) as f64) * 0.001953125f64, // 0.001953125 = 2^-9
        _ => ((sign * (8 + mant) as i8) as f64) * 2.0f64.powi(exp as i32 - 10), // 10 = 7 (bias) + 3 (mantissa bits)
    }
}

pub struct E4M3Translator {}

impl NumericTranslator<VarId, ScopeId> for E4M3Translator {
    fn name(&self) -> String {
        String::from("FP: 8-bit (E4M3)")
    }

    fn numeric_translate(
        &self,
        _: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        v.to_u8()
            .map(decode_e4m3)
            .map(NumericalValueRepr::FloatingPoint)
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (0.0, 1.0) // TODO
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, 8)
    }
}

pub struct UnsignedFixedPointTranslator;

impl NumericTranslator<VarId, ScopeId> for UnsignedFixedPointTranslator {
    fn name(&self) -> String {
        "Unsigned fixed point".into()
    }

    fn numeric_translate(
        &self,
        meta: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        Some(NumericalValueRepr::FixedPoint {
            int: BigInt::from(v.clone()),
            scaling_factor: meta.index.map(|idx| -idx.lsb).unwrap_or(0) as i32,
        })
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (0.0, 1.0) // TODO
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        let candidates = ["unresolved_ufixed".to_string(), "ufixed".to_string()];
        if match_variable_type_name(&variable.variable_type_name, &candidates) {
            Ok(TranslationPreference::Prefer)
        } else {
            translates_all_bit_types(variable)
        }
    }
}

pub struct SignedFixedPointTranslator;

impl NumericTranslator<VarId, ScopeId> for SignedFixedPointTranslator {
    fn name(&self) -> String {
        "Signed fixed point".into()
    }

    fn numeric_translate(
        &self,
        meta: &VariableMeta<VarId, ScopeId>,
        v: &BigUint,
    ) -> Option<NumericalValueRepr> {
        Some(NumericalValueRepr::FixedPoint {
            int: biguint_signed(v, meta.num_bits.unwrap()),
            scaling_factor: meta.index.map(|idx| -idx.lsb).unwrap_or(0) as i32,
        })
    }

    fn variable_range(&self, _: &VariableMeta<VarId, ScopeId>) -> (f64, f64) {
        (0.0, 1.0) // TODO
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        let candidates = ["unresolved_sfixed".to_string(), "sfixed".to_string()];
        if match_variable_type_name(&variable.variable_type_name, &candidates) {
            Ok(TranslationPreference::Prefer)
        } else {
            translates_all_bit_types(variable)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use surfer_translation_types::VariableValue;

    #[test]
    fn signed_translation_from_string() {
        assert_eq!(
            SignedTranslator {}
                .basic_translate(5, &VariableValue::String("10000".to_string()))
                .0,
            "-16"
        );

        assert_eq!(
            SignedTranslator {}
                .basic_translate(5, &VariableValue::String("01000".to_string()))
                .0,
            "8"
        );
    }

    #[test]
    fn signed_translation_from_biguint() {
        assert_eq!(
            SignedTranslator {}
                .basic_translate(5, &VariableValue::BigUint(BigUint::from(0b10011u32)))
                .0,
            "-13"
        );

        assert_eq!(
            SignedTranslator {}
                .basic_translate(5, &VariableValue::BigUint(BigUint::from(0b01000u32)))
                .0,
            "8"
        );
        assert_eq!(
            SignedTranslator {}
                .basic_translate(2, &VariableValue::BigUint(BigUint::from(0u32)))
                .0,
            "0"
        );
    }

    #[test]
    fn unsigned_translation_from_string() {
        assert_eq!(
            UnsignedTranslator {}
                .basic_translate(5, &VariableValue::String("10000".to_string()))
                .0,
            "16"
        );

        assert_eq!(
            UnsignedTranslator {}
                .basic_translate(5, &VariableValue::String("01000".to_string()))
                .0,
            "8"
        );
    }

    #[test]
    fn unsigned_translation_from_biguint() {
        assert_eq!(
            UnsignedTranslator {}
                .basic_translate(5, &VariableValue::BigUint(BigUint::from(0b10011u32)))
                .0,
            "19"
        );

        assert_eq!(
            UnsignedTranslator {}
                .basic_translate(5, &VariableValue::BigUint(BigUint::from(0b01000u32)))
                .0,
            "8"
        );
        assert_eq!(
            UnsignedTranslator {}
                .basic_translate(2, &VariableValue::BigUint(BigUint::from(0u32)))
                .0,
            "0"
        );
    }

    #[test]
    fn e4m3_translation_from_biguint() {
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::BigUint(BigUint::from(0b10001000u8)))
                .0,
            "-0.015625"
        );
    }

    #[test]
    fn e4m3_translation_from_string() {
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::String("11111111".to_string()))
                .0,
            "NaN"
        );
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::String("00000011".to_string()))
                .0,
            "0.005859375"
        );
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::String("10000000".to_string()))
                .0,
            "-0"
        );
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::String("00000000".to_string()))
                .0,
            "0"
        );
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::String("11110111".to_string()))
                .0,
            "-240"
        );
        assert_eq!(
            E4M3Translator {}
                .basic_translate(8, &VariableValue::String("01000000".to_string()))
                .0,
            "2"
        );
    }

    #[test]
    fn e5m2_translation_from_biguint() {
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::BigUint(BigUint::from(0b10000100u8)))
                .0,
            "-6.1035156e-5"
        );
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::BigUint(BigUint::from(0b11111100u8)))
                .0,
            "∞"
        );
    }

    #[test]
    fn e5m2_translation_from_string() {
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::String("11111111".to_string()))
                .0,
            "NaN"
        );
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::String("00000011".to_string()))
                .0,
            "4.5776367e-5"
        );
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::String("10000000".to_string()))
                .0,
            "-0"
        );
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::String("00000000".to_string()))
                .0,
            "0"
        );
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::String("11111011".to_string()))
                .0,
            "-57344"
        );
        assert_eq!(
            E5M2Translator {}
                .basic_translate(8, &VariableValue::String("01000000".to_string()))
                .0,
            "2"
        );
    }

    #[test]
    fn posit8_translation_from_biguint() {
        assert_eq!(
            Posit8Translator {}
                .basic_translate(8, &VariableValue::BigUint(BigUint::from(0b10001000u8)))
                .0,
            "-8"
        );
        assert_eq!(
            Posit8Translator {}
                .basic_translate(8, &VariableValue::BigUint(BigUint::from(0u8)))
                .0,
            "0"
        );
    }

    #[test]
    fn posit8_translation_from_string() {
        assert_eq!(
            Posit8Translator {}
                .basic_translate(8, &VariableValue::String("11111111".to_string()))
                .0,
            "-0.015625"
        );
        assert_eq!(
            Posit8Translator {}
                .basic_translate(8, &VariableValue::String("00000011".to_string()))
                .0,
            "0.046875"
        );
        assert_eq!(
            Posit8Translator {}
                .basic_translate(8, &VariableValue::String("10000000".to_string()))
                .0,
            "NaN"
        );
    }

    #[test]
    fn posit16_translation_from_biguint() {
        assert_eq!(
            Posit16Translator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b1010101010001000u16))
                )
                .0,
            "-2.68359375"
        );
        assert_eq!(
            Posit16Translator {}
                .basic_translate(16, &VariableValue::BigUint(BigUint::from(0u16)))
                .0,
            "0"
        );
    }

    #[test]
    fn posit16_translation_from_string() {
        assert_eq!(
            Posit16Translator {}
                .basic_translate(16, &VariableValue::String("1111111111111111".to_string()))
                .0,
            "-0.000000003725290298461914"
        );
        assert_eq!(
            Posit16Translator {}
                .basic_translate(16, &VariableValue::String("0111000000000011".to_string()))
                .0,
            "16.046875"
        );
        assert_eq!(
            Posit16Translator {}
                .basic_translate(16, &VariableValue::String("1000000000000000".to_string()))
                .0,
            "NaN"
        );
    }

    #[test]
    fn posit32_translation_from_biguint() {
        assert_eq!(
            Posit32Translator {}
                .basic_translate(
                    32,
                    &VariableValue::BigUint(BigUint::from(0b1010101010001000u16))
                )
                .0,
            "0.0000000000000000023056236824262055"
        );
        assert_eq!(
            Posit32Translator {}
                .basic_translate(32, &VariableValue::BigUint(BigUint::from(0u32)))
                .0,
            "0"
        );
    }

    #[test]
    fn posit32_translation_from_string() {
        assert_eq!(
            Posit32Translator {}
                .basic_translate(
                    32,
                    &VariableValue::String("10000111000000001111111111111111".to_string())
                )
                .0,
            "-8176.000244140625"
        );
        assert_eq!(
            Posit32Translator {}
                .basic_translate(
                    32,
                    &VariableValue::String("01110000000000111000000000000000".to_string())
                )
                .0,
            "257.75"
        );
    }

    #[test]
    fn quire8_translation_from_biguint() {
        assert_eq!(
            PositQuire8Translator {}
                .basic_translate(
                    32,
                    &VariableValue::BigUint(BigUint::from(0b1010101010001000u16))
                )
                .0,
            "10"
        );
        assert_eq!(
            PositQuire8Translator {}
                .basic_translate(32, &VariableValue::BigUint(BigUint::from(0u16)))
                .0,
            "0"
        );
    }

    #[test]
    fn quire8_translation_from_string() {
        assert_eq!(
            PositQuire8Translator {}
                .basic_translate(
                    32,
                    &VariableValue::String("10000111000000001111111111111111".to_string())
                )
                .0,
            "-64"
        );
        assert_eq!(
            PositQuire8Translator {}
                .basic_translate(
                    32,
                    &VariableValue::String("01110000000000111000000000000000".to_string())
                )
                .0,
            "64"
        );
    }

    #[test]
    fn quire16_translation_from_biguint() {
        assert_eq!(
            PositQuire16Translator {}
                .basic_translate(128, &VariableValue::BigUint(BigUint::from(0b10101010100010001010101010001000101010101000100010101010100010001010101010001000101010101000100010101010100010001010101010001000u128)))
                .0,
            "-268435456"
        );
        assert_eq!(
            PositQuire16Translator {}
                .basic_translate(128, &VariableValue::BigUint(BigUint::from(7u8)))
                .0,
            "0.000000003725290298461914"
        );
        assert_eq!(
            PositQuire16Translator {}
                .basic_translate(128, &VariableValue::BigUint(BigUint::from(0u8)))
                .0,
            "0"
        );
    }

    #[test]
    fn quire16_translation_from_string() {
        assert_eq!(
            PositQuire16Translator {}
                .basic_translate(
                    128,
                    &VariableValue::String(
                        "1000011100000000111111111111111101110000000000111000000000000000"
                            .to_string()
                    )
                )
                .0,
            "135"
        );
        assert_eq!(
            PositQuire16Translator {}
                .basic_translate(
                    128,
                    &VariableValue::String("01110000000000111000000000000000".to_string())
                )
                .0,
            "0.000000029802322387695313"
        );
    }

    #[test]
    fn bloat16_translation_from_string() {
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("0100100011100011".to_string()))
                .0,
            "464896"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("1000000000000000".to_string()))
                .0,
            "-0"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("1111111111111111".to_string()))
                .0,
            "NaN"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("01001z0011100011".to_string()))
                .0,
            "HIGHIMP"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("01001q0011100011".to_string()))
                .0,
            "UNKNOWN VALUES"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("01001-0011100011".to_string()))
                .0,
            "DON'T CARE"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("01001w0011100011".to_string()))
                .0,
            "UNDEF WEAK"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("01001h0011100011".to_string()))
                .0,
            "WEAK"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(16, &VariableValue::String("01001u0011100011".to_string()))
                .0,
            "UNDEF"
        );
    }

    #[test]
    fn bloat16_translation_from_bigunit() {
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b1010101010001000u16))
                )
                .0,
            "-2.4158453e-13"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b1000000000000000u16))
                )
                .0,
            "-0"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b0000000000000000u16))
                )
                .0,
            "0"
        );
        assert_eq!(
            BFloat16Translator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b1111111111111111u16))
                )
                .0,
            "NaN"
        );
    }

    #[test]
    fn half_translation_from_biguint() {
        assert_eq!(
            HalfPrecisionTranslator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b1000000000000000u16))
                )
                .0,
            "-0"
        );
        assert_eq!(
            HalfPrecisionTranslator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b0000000000000000u16))
                )
                .0,
            "0"
        );
        assert_eq!(
            HalfPrecisionTranslator {}
                .basic_translate(
                    16,
                    &VariableValue::BigUint(BigUint::from(0b1111111111111111u16))
                )
                .0,
            "NaN"
        );
    }

    #[test]
    fn half_translation_from_string() {
        assert_eq!(
            HalfPrecisionTranslator {}
                .basic_translate(16, &VariableValue::String("0100100011100011".to_string()))
                .0,
            "9.7734375"
        );
        assert_eq!(
            HalfPrecisionTranslator {}
                .basic_translate(16, &VariableValue::String("1000000000000000".to_string()))
                .0,
            "-0"
        );
        assert_eq!(
            HalfPrecisionTranslator {}
                .basic_translate(16, &VariableValue::String("1111111111111111".to_string()))
                .0,
            "NaN"
        );
    }

    #[test]
    fn single_translation_from_bigunit() {
        assert_eq!(
            SinglePrecisionTranslator {}
                .basic_translate(
                    32,
                    &VariableValue::BigUint(BigUint::from(0b01010101010001001010101010001000u32))
                )
                .0,
            "1.3514794e13"
        );
        assert_eq!(
            SinglePrecisionTranslator {}
                .basic_translate(
                    32,
                    &VariableValue::BigUint(BigUint::from(0b10000000000000000000000000000000u32))
                )
                .0,
            "-0"
        );
        assert_eq!(
            SinglePrecisionTranslator {}
                .basic_translate(
                    32,
                    &VariableValue::BigUint(BigUint::from(0b00000000000000000000000000000000u32))
                )
                .0,
            "0"
        );
        assert_eq!(
            SinglePrecisionTranslator {}
                .basic_translate(
                    32,
                    &VariableValue::BigUint(BigUint::from(0b11111111111111111111111111111111u32))
                )
                .0,
            "NaN"
        );
    }

    #[test]
    fn double_translation_from_bigunit() {
        assert_eq!(
            DoublePrecisionTranslator {}
                .basic_translate(
                    64,
                    &VariableValue::BigUint(BigUint::from(
                        0b0101010101000100101010101000100001010101010001001010101010001000u64
                    ))
                )
                .0,
            "5.785860578429741e102"
        );
        assert_eq!(
            DoublePrecisionTranslator {}
                .basic_translate(
                    64,
                    &VariableValue::BigUint(BigUint::from(
                        0b1000000000000000000000000000000000000000000000000000000000000000u64
                    ))
                )
                .0,
            "-0"
        );
        assert_eq!(
            DoublePrecisionTranslator {}
                .basic_translate(
                    64,
                    &VariableValue::BigUint(BigUint::from(
                        0b0000000000000000000000000000000000000000000000000000000000000000u64
                    ))
                )
                .0,
            "0"
        );
        assert_eq!(
            DoublePrecisionTranslator {}
                .basic_translate(
                    64,
                    &VariableValue::BigUint(BigUint::from(
                        0b1111111111111111111111111111111111111111111111111111111111111111u64
                    ))
                )
                .0,
            "NaN"
        );
    }
}
