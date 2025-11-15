use crate::signal_rmq::SignalRMQ;
use crate::translation::DynTranslator;
use crate::wave_container::VariableMeta;
use crate::wellen::{convert_variable_value, SignalAccessor};
use surfer_translation_types::{ValueRepr, VariableValue};

/// Cache entry for a single analog signal
/// Keyed by (SignalRef, translator_name) in the parent HashMap
#[derive(Clone, Debug)]
pub struct AnalogSignalCache {
    /// SignalRMQ structure for fast min/max queries
    pub rmq: SignalRMQ,

    /// Global min/max across entire signal (from RMQ)
    pub global_min: f64,
    pub global_max: f64,

    /// Total number of time units in the signal (for cache invalidation)
    pub num_timestamps: u64,
}

impl AnalogSignalCache {
    /// Build cache for an analog signal
    ///
    /// # Arguments
    /// * `accessor` - SignalAccessor providing signal data
    /// * `translator` - Translator for value formatting
    /// * `meta` - Variable metadata
    /// * `num_timestamps` - Total time units in waveform
    /// * `block_size` - RMQ block size (default: 64)
    ///
    /// # Returns
    /// Complete cache with RMQ structure, or None if unsuitable
    pub fn build(
        accessor: SignalAccessor,
        translator: &DynTranslator,
        meta: &VariableMeta,
        num_timestamps: u64,
        block_size: Option<usize>,
    ) -> Option<Self> {
        let block_size = block_size.unwrap_or(64);

        let mut signal_data = Vec::new();

        // Iterate through all signal changes using wellen's iter_changes
        let signal = accessor.signal();
        let time_table = accessor.time_table();

        for (time_idx, signal_value) in signal.iter_changes() {
            let time_u64 = *time_table.get(time_idx as usize)?;
            let var_value = convert_variable_value(signal_value);
            let numeric = translate_to_numeric(translator, meta, &var_value).unwrap_or(f64::NAN);
            signal_data.push((time_u64, numeric));
        }

        if signal_data.is_empty() {
            return None;
        }

        // Build RMQ structure
        let rmq = SignalRMQ::new(signal_data, block_size);

        // Extract global min/max from first full-range query
        let (first_time, last_time) = rmq.time_range()?;
        let global = rmq.query_time_range(first_time, last_time)?;

        Some(Self {
            rmq,
            global_min: global.min,
            global_max: global.max,
            num_timestamps,
        })
    }

    /// Query min/max for a time range
    pub fn query_time_range(&self, start: u64, end: u64) -> Option<(f64, f64)> {
        let result = self.rmq.query_time_range(start, end)?;
        Some((result.min, result.max))
    }
}

/// Translate a variable value to a numeric f64 using the given translator
/// Returns None if translation fails or value cannot be parsed as numeric
pub fn translate_to_numeric(
    translator: &DynTranslator,
    meta: &VariableMeta,
    value: &VariableValue,
) -> Option<f64> {
    let translation = translator.translate(meta, value).ok()?;
    let value_str = match &translation.val {
        ValueRepr::Bit(c) => c.to_string(),
        ValueRepr::Bits(_, s) => s.clone(),
        ValueRepr::String(s) => s.clone(),
        _ => return None,
    };
    parse_numeric_value(&value_str, &translator.name())
}

/// Parse a numeric value from a string based on translator type
/// Trusts the translator to determine the parsing format
pub fn parse_numeric_value(s: &str, translator_name: &str) -> Option<f64> {
    let s = s.trim();

    // Determine parsing strategy based on translator type
    let translator_lower = translator_name.to_lowercase();

    if translator_lower.contains("hex") {
        // Hex translator: parse as hexadecimal (handle optional 0x prefix)
        let hex_str = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
        u64::from_str_radix(hex_str, 16).ok().map(|v| v as f64)
    } else if translator_lower.contains("bin") {
        // Binary translator: parse as binary (handle optional 0b prefix)
        let bin_str = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")).unwrap_or(s);
        u64::from_str_radix(bin_str, 2).ok().map(|v| v as f64)
    } else {
        // Decimal-based translators (Unsigned, Signed, Float, etc.): parse as decimal/float
        if let Ok(v) = s.parse::<f64>() {
            return Some(v);
        }

        // Fall back to hex for values that can't be parsed as decimal (e.g., "f9", "ca")
        // This handles misconfigured translators returning hex without being marked as Hex
        u64::from_str_radix(s, 16).ok().map(|v| v as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_numeric_value() {
        // Hex translator - unprefixed
        assert_eq!(parse_numeric_value("f9", "Hex"), Some(249.0));
        assert_eq!(parse_numeric_value("ca", "Hexadecimal"), Some(202.0));
        assert_eq!(parse_numeric_value("80", "Hex"), Some(128.0));
        assert_eq!(parse_numeric_value("10", "Hex"), Some(16.0));

        // Hex translator - with 0x prefix (should be stripped)
        assert_eq!(parse_numeric_value("0x10", "Hex"), Some(16.0));
        assert_eq!(parse_numeric_value("0xFF", "Hexadecimal"), Some(255.0));

        // Decimal translators
        assert_eq!(parse_numeric_value("123", "Unsigned"), Some(123.0));
        assert_eq!(parse_numeric_value("123.45", "Float"), Some(123.45));
        assert_eq!(parse_numeric_value("80", "Unsigned"), Some(80.0));
        assert_eq!(parse_numeric_value("10", "Signed"), Some(10.0));

        // Scientific notation with Float translator
        assert_eq!(parse_numeric_value("1.5e3", "Float"), Some(1500.0));
        assert_eq!(parse_numeric_value("-3.14e-2", "Float"), Some(-0.0314));

        // Binary translator
        assert_eq!(parse_numeric_value("1010", "Binary"), Some(10.0));
        assert_eq!(parse_numeric_value("0b1010", "Binary"), Some(10.0));
        assert_eq!(parse_numeric_value("11111111", "Bin"), Some(255.0));

        // Fallback: hex values with decimal translators (misconfiguration)
        assert_eq!(parse_numeric_value("f9", "Unsigned"), Some(249.0));
        assert_eq!(parse_numeric_value("ca", "Signed"), Some(202.0));

        // Invalid values
        assert_eq!(parse_numeric_value("xyz", "Hex"), None);
        assert_eq!(parse_numeric_value("invalid", "Unsigned"), None);
        assert_eq!(parse_numeric_value("12", "Binary"), None); // '2' is not valid binary
    }
}
