use ecolor::Color32;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use surfer_translation_types::{
    extend_string, BasicTranslator, TranslationPreference, ValueKind, VariableValue,
};
use thiserror::Error;

use crate::{
    translation::{check_single_wordlength, kind_for_binary_representation},
    wave_container::{ScopeId, VarId, VariableMeta},
};

static KIND_COLOR_KEYWORDS: OnceLock<HashMap<&'static str, ValueKind>> = OnceLock::new();

fn kind_color_keywords() -> &'static HashMap<&'static str, ValueKind> {
    KIND_COLOR_KEYWORDS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("normal", ValueKind::Normal);
        m.insert("default", ValueKind::Normal);
        m.insert("undef", ValueKind::Undef);
        m.insert("highimp", ValueKind::HighImp);
        m.insert("warn", ValueKind::Warn);
        m.insert("dontcare", ValueKind::DontCare);
        m.insert("weak", ValueKind::Weak);
        m.insert("error", ValueKind::Error);
        m.insert("black", ValueKind::Custom(Color32::BLACK));
        m.insert("white", ValueKind::Custom(Color32::WHITE));
        m.insert("red", ValueKind::Custom(Color32::RED));
        m.insert("green", ValueKind::Custom(Color32::GREEN));
        m.insert("blue", ValueKind::Custom(Color32::BLUE));
        m.insert("yellow", ValueKind::Custom(Color32::YELLOW));
        m.insert("cyan", ValueKind::Custom(Color32::CYAN));
        m.insert("magenta", ValueKind::Custom(Color32::MAGENTA));
        m.insert("gray", ValueKind::Custom(Color32::GRAY));
        m.insert("grey", ValueKind::Custom(Color32::GRAY));
        m.insert("light_gray", ValueKind::Custom(Color32::LIGHT_GRAY));
        m.insert("light_grey", ValueKind::Custom(Color32::LIGHT_GRAY));
        m.insert("dark_gray", ValueKind::Custom(Color32::DARK_GRAY));
        m.insert("dark_grey", ValueKind::Custom(Color32::DARK_GRAY));
        m.insert("brown", ValueKind::Custom(Color32::BROWN));
        m.insert("dark_red", ValueKind::Custom(Color32::DARK_RED));
        m.insert("light_red", ValueKind::Custom(Color32::LIGHT_RED));
        m.insert("orange", ValueKind::Custom(Color32::ORANGE));
        m.insert("light_yellow", ValueKind::Custom(Color32::LIGHT_YELLOW));
        m.insert("khaki", ValueKind::Custom(Color32::KHAKI));
        m.insert("dark_green", ValueKind::Custom(Color32::DARK_GREEN));
        m.insert("light_green", ValueKind::Custom(Color32::LIGHT_GREEN));
        m.insert("dark_blue", ValueKind::Custom(Color32::DARK_BLUE));
        m.insert("light_blue", ValueKind::Custom(Color32::LIGHT_BLUE));
        m.insert("purple", ValueKind::Custom(Color32::PURPLE));
        m.insert("gold", ValueKind::Custom(Color32::GOLD));
        m
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct MnemonicEntry {
    pub label: String,
    pub kind: ValueKind,
}

#[derive(Debug, Clone)]
pub struct MnemonicMap {
    pub name: Option<String>,
    pub bits: u64,
    pub entries: HashMap<String, MnemonicEntry>,
}

pub struct MnemonicTranslator {
    pub map: MnemonicMap,
}

#[derive(Debug, Error)]
pub enum MnemonicParseError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid Bits: line format")]
    InvalidBitsLineFormat,

    #[error("Invalid bits value: {0}")]
    InvalidBitsValue(String),

    #[error("Invalid hex number: {0}")]
    InvalidHex(String),

    #[error("Unknown kind/color: {0}")]
    UnknownKindColor(String),

    #[error(
        "Binary string '{value}' requires {required} bits, but only {specified} bits specified"
    )]
    BinaryTooWide {
        value: String,
        required: usize,
        specified: usize,
    },

    #[error("String '{value}' has {value_len} characters, expected {expected} characters to match bit width")]
    StringLengthMismatch {
        value: String,
        value_len: usize,
        expected: usize,
    },

    #[error("Missing mnemonic")]
    MissingSecondColumn,

    #[error("Empty line")]
    EmptyLine,

    #[error("Line {line}: {message}\n  Content: {content}")]
    LineError {
        line: usize,
        content: String,
        message: String,
    },
}

impl MnemonicTranslator {
    pub fn new_from_file<P: AsRef<Path>>(path: P) -> Result<Self, MnemonicParseError> {
        Ok(MnemonicTranslator {
            map: MnemonicMap::new(path)?,
        })
    }
}

impl BasicTranslator<VarId, ScopeId> for MnemonicTranslator {
    fn name(&self) -> String {
        self.map
            .name
            .clone()
            .unwrap_or_else(|| "Mnemonic".to_string())
    }

    fn basic_translate(&self, num_bits: u64, value: &VariableValue) -> (String, ValueKind) {
        let var_string = match value {
            VariableValue::BigUint(v) => format!("{v:0width$b}", width = num_bits as usize),
            VariableValue::String(s) => {
                format!("{extra_bits}{s}", extra_bits = extend_string(s, num_bits))
            }
        };

        if let Some(entry) = self.map.entries.get(&var_string) {
            (entry.label.clone(), entry.kind)
        } else {
            let val_kind = kind_for_binary_representation(&var_string);
            (var_string, val_kind)
        }
    }

    fn translates(&self, variable: &VariableMeta) -> eyre::Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, self.map.bits as u32)
    }
}

impl MnemonicMap {
    pub fn new<P: AsRef<Path>>(file: P) -> Result<Self, MnemonicParseError> {
        parse_file(file)
    }
}

pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<MnemonicMap, MnemonicParseError> {
    let content = fs::read_to_string(&path)?;

    // Extract filename (without extension) for default name
    let default_name = path
        .as_ref()
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    parse_content_with_default_name(&content, default_name)
}

pub fn parse_content_with_default_name(
    content: &str,
    default_name: Option<String>,
) -> Result<MnemonicMap, MnemonicParseError> {
    let lines_iter = content.lines().enumerate();

    let mut name = None;
    let mut bits = None;
    let mut raw_entries = Vec::new();

    // Process all lines using iterator
    for (line_num, line_str) in lines_iter {
        let trimmed = line_str.trim();
        let processed = strip_inline_comment(trimmed).trim();

        // Skip empty lines and comments
        if processed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }

        // Check for Name: line if we haven't found one yet
        if starts_with_ignore_ascii_case(processed, "name:") {
            if name.is_some() {
                return Err(MnemonicParseError::LineError {
                    line: line_num + 1,
                    content: trimmed.to_string(),
                    message: "Multiple Name specifiers found".to_string(),
                });
            }
            name = Some(processed[5..].trim().to_string());
            continue;
        }

        // Check for Bits: line if we haven't found one yet
        if starts_with_ignore_ascii_case(processed, "bits:") {
            if bits.is_some() {
                return Err(MnemonicParseError::LineError {
                    line: line_num + 1,
                    content: trimmed.to_string(),
                    message: "Multiple Bits specifiers found".to_string(),
                });
            }
            bits = Some(parse_bits_line(processed)?);
            continue;
        }

        match parse_line(processed) {
            Ok(entry) => raw_entries.push(entry),
            Err(e) => {
                return Err(MnemonicParseError::LineError {
                    line: line_num + 1,
                    content: trimmed.to_string(),
                    message: e.to_string(),
                })
            }
        }
    }

    // Determine bit width if not provided
    let bit_width = if let Some(b) = bits {
        b as usize
    } else {
        // Find the longest bit string
        raw_entries
            .iter()
            .map(|entry| entry.0.len())
            .max()
            .unwrap_or(0)
    };

    // Validate and normalize all entries, building HashMap
    let mut entries = HashMap::new();
    for (first, second, color) in raw_entries {
        let normalized_first = normalize_first_column(&first, bit_width)?;
        if entries.contains_key(&normalized_first) {
            tracing::warn!(
                "Duplicate mnemonic key '{}' encountered; keeping first occurrence",
                normalized_first
            );
            continue;
        }
        entries.insert(
            normalized_first,
            MnemonicEntry {
                label: second,
                kind: color,
            },
        );
    }

    // Use default_name if no name was found in the file
    let final_name = name.or(default_name);

    Ok(MnemonicMap {
        name: final_name,
        bits: bit_width as u64,
        entries,
    })
}

fn parse_bits_line(line: &str) -> Result<u32, MnemonicParseError> {
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(MnemonicParseError::InvalidBitsLineFormat);
    }

    let bits_str = parts[1].trim();
    bits_str
        .parse::<u32>()
        .map_err(|_| MnemonicParseError::InvalidBitsValue(bits_str.to_string()))
}

fn normalize_first_column(first: &str, bit_width: usize) -> Result<String, MnemonicParseError> {
    // Check if the first column contains only '0' and '1' (binary string)
    let is_binary = first.chars().all(|c| c == '0' || c == '1');

    if is_binary {
        // Pad binary string to match bit_width
        let padded = pad_binary_string(first, bit_width);
        if padded.len() > bit_width {
            return Err(MnemonicParseError::BinaryTooWide {
                value: first.to_string(),
                required: padded.len(),
                specified: bit_width,
            });
        }
        Ok(padded)
    } else {
        // Regular string, validate it matches bit_width
        if first.len() != bit_width {
            return Err(MnemonicParseError::StringLengthMismatch {
                value: first.to_string(),
                value_len: first.len(),
                expected: bit_width,
            });
        }
        Ok(first.to_string().to_lowercase())
    }
}

fn parse_line(line: &str) -> Result<(String, String, ValueKind), MnemonicParseError> {
    let tokens = split_tokens(line);

    if tokens.is_empty() {
        return Err(MnemonicParseError::EmptyLine);
    }

    let first = parse_first_column(&tokens[0])?;

    if tokens.len() < 2 {
        return Err(MnemonicParseError::MissingSecondColumn);
    }

    let second = tokens[1].clone();

    let kind = if tokens.len() >= 3 {
        parse_color_kind(&tokens[2])?
    } else {
        ValueKind::Normal
    };

    Ok((first, second, kind))
}

fn parse_first_column(token: &str) -> Result<String, MnemonicParseError> {
    // Support underscore separators; only strip them for numeric parsing, keep original for literals
    let cleaned = token.replace('_', "");

    if cleaned.starts_with("0x") || cleaned.starts_with("0X") {
        let hex_str = &cleaned[2..];
        let num = u64::from_str_radix(hex_str, 16)
            .map_err(|_| MnemonicParseError::InvalidHex(token.to_string()))?;
        return Ok(format!("{:b}", num));
    }

    if !cleaned.is_empty() && cleaned.chars().all(|c| c == '0' || c == '1') {
        return Ok(cleaned);
    }

    if let Ok(num) = cleaned.parse::<u64>() {
        return Ok(format!("{:b}", num));
    }

    Ok(token.to_string())
}

fn pad_binary_string(binary: &str, width: usize) -> String {
    format!("{:0>width$}", binary, width = width)
}

fn parse_color_kind(token: &str) -> Result<ValueKind, MnemonicParseError> {
    // Try hex color (#RRGGBB or RRGGBB)
    let hex_str = token.strip_prefix('#').unwrap_or(token);

    if hex_str.len() == 6 && hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        let r = u8::from_str_radix(&hex_str[0..2], 16)
            .map_err(|_| MnemonicParseError::InvalidHex(token.to_string()))?;
        let g = u8::from_str_radix(&hex_str[2..4], 16)
            .map_err(|_| MnemonicParseError::InvalidHex(token.to_string()))?;
        let b = u8::from_str_radix(&hex_str[4..6], 16)
            .map_err(|_| MnemonicParseError::InvalidHex(token.to_string()))?;
        return Ok(ValueKind::Custom(Color32::from_rgb(r, g, b)));
    }

    // Try value kinds and ecolor::Color32 named colors using lookup table
    let lower = token.to_lowercase();
    kind_color_keywords()
        .get(lower.as_str())
        .copied()
        .ok_or_else(|| MnemonicParseError::UnknownKindColor(token.to_string()))
}

fn split_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars();

    while let Some(ch) = chars.next() {
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }

        if ch == '\\' && in_quotes {
            // Support escaped characters inside quoted strings (e.g. \" and \\\\)
            if let Some(escaped) = chars.next() {
                current.push(escaped);
            }
            continue;
        }

        if ch == ' ' && !in_quotes {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            continue;
        }

        current.push(ch);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut i = 0;
    while i + 1 < bytes.len() {
        match bytes[i] as char {
            '"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            '/' if !in_quotes && bytes[i + 1] as char == '/' => {
                // Found // outside quotes
                return &line[..i];
            }
            _ => i += 1,
        }
    }
    line
}

fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use num::BigUint;

    fn color_eq(a: Color32, b: Color32) -> bool {
        a.r() == b.r() && a.g() == b.g() && a.b() == b.b() && a.a() == b.a()
    }

    #[test]
    fn parse_empty_content_uses_default_name_and_no_bits() {
        let map = parse_content_with_default_name("", Some("DefaultName".to_string())).unwrap();
        assert_eq!(map.name, Some("DefaultName".to_string()));
        assert_eq!(map.bits, 0); // empty content => no bits
        assert!(map.entries.is_empty());
    }

    #[test]
    fn parse_with_name_bits_and_entries_binary_and_hex_and_colors() {
        let content = "Name: MyMap\nBits: 4\n0 ZERO red\n1 ONE #00FF00\n0xA TEN blue";
        let map = parse_content_with_default_name(content, None).unwrap();
        assert_eq!(map.name, Some("MyMap".to_string()));
        assert_eq!(map.bits, 4);
        // Expect padded binary keys
        let zero = map.entries.get("0000").expect("ZERO entry");
        assert_eq!(zero.label, "ZERO");
        assert_eq!(zero.kind, ValueKind::Custom(Color32::RED));
        let one = map.entries.get("0001").expect("ONE entry");
        assert_eq!(one.label, "ONE");
        // #00FF00 => green
        assert_eq!(one.kind, ValueKind::Custom(Color32::GREEN));
        let ten = map.entries.get("1010").expect("TEN entry");
        assert_eq!(ten.label, "TEN");
        assert_eq!(ten.kind, ValueKind::Custom(Color32::BLUE));
    }

    #[test]
    fn duplicate_keys_keep_first_and_log() {
        let content = "Bits: 4\n0 ZERO red\n0 ZERO_DUP blue\n1 ONE green";
        let map = parse_content_with_default_name(content, None).unwrap();
        // Only one entry for key 0000
        assert_eq!(map.entries.get("0000").unwrap().label, "ZERO");
        assert_eq!(map.entries.get("0001").unwrap().label, "ONE");
        assert_eq!(map.entries.len(), 2);
    }

    #[test]
    fn infer_bit_width_from_longest_entry() {
        // Longest entry after normalization should determine width
        let content = "3 THREE\n15 FIFTEEN"; // 3 => 11, 15 => 1111 so width=4
        let map = parse_content_with_default_name(content, Some("Numbers".to_string())).unwrap();
        assert_eq!(map.bits, 4);
        assert!(map.entries.contains_key("0011"));
        assert!(map.entries.contains_key("1111"));
    }

    #[test]
    fn error_on_mismatched_string_length() {
        // Bits:3 but entry has 4 chars in first column
        let content = "Bits: 3\nxxuu LABEL";
        let err = parse_content_with_default_name(content, None).unwrap_err();
        match err {
            MnemonicParseError::StringLengthMismatch { expected, .. } => {
                assert_eq!(expected, 3)
            }
            other => panic!("Unexpected error variant: {other}"),
        }
    }

    #[test]
    fn split_tokens_handles_quotes() {
        let line = "0101 \"Label With Spaces\" red";
        let tokens = split_tokens(line);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], "0101");
        assert_eq!(tokens[1], "Label With Spaces");
        assert_eq!(tokens[2], "red");
    }

    #[test]
    fn split_tokens_handles_escaped_quotes_and_backslashes() {
        // Use a raw string literal so escapes are visible to the parser
        let line = r#"0101 "Label \"With\" Escaped\\Back" red"#;
        let tokens = split_tokens(line);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], "0101");
        // Expected: the escaped quotes become literal quotes, and \\ becomes a single backslash
        assert_eq!(tokens[1], "Label \"With\" Escaped\\Back");
        assert_eq!(tokens[2], "red");
    }

    #[test]
    fn parse_hex_and_decimal_numbers() {
        let content = "Bits: 5\n0x1F HEXVAL blue\n7 DECVAL green"; // 0x1F => 11111, 7 => 00111
        let map = parse_content_with_default_name(content, None).unwrap();
        assert_eq!(map.bits, 5);
        assert!(map.entries.contains_key("11111"));
        assert!(map.entries.contains_key("00111"));
        assert_eq!(map.entries.get("11111").unwrap().label, "HEXVAL");
        assert_eq!(map.entries.get("00111").unwrap().label, "DECVAL");
    }

    #[test]
    fn mix_binary_and_decimal_without_bits_line_infers_width() {
        // Raw binary token plus decimal numbers; longest normalized width should be 4
        let content = "0101 BINLABEL\n7 DECSEVEN\n13 DECTHIRTEEN"; // 7 => 111, 13 => 1101
        let map = parse_content_with_default_name(content, Some("Mixed".to_string())).unwrap();
        assert_eq!(map.name, Some("Mixed".to_string()));
        assert_eq!(map.bits, 4);
        assert!(map.entries.contains_key("0101")); // binary preserved
        assert!(map.entries.contains_key("0111")); // 7 padded to 4 bits
        assert!(map.entries.contains_key("1101")); // 13 already 4 bits
        assert_eq!(map.entries.get("0101").unwrap().label, "BINLABEL");
        assert_eq!(map.entries.get("0111").unwrap().label, "DECSEVEN");
        assert_eq!(map.entries.get("1101").unwrap().label, "DECTHIRTEEN");
    }

    #[test]
    fn file_based_translator_basic_translate_and_fallback() {
        use std::time::{SystemTime, UNIX_EPOCH};
        // Unique temp file path
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!("mnemonic_test_{}.txt", ts));

        let content = "Name: ExampleMnemonic\nBits: 4\n0 ZERO red\n1 ONE green\n10 TWO blue";
        std::fs::write(&path, content).expect("write mnemonic file");

        let translator = MnemonicTranslator::new_from_file(&path)
            .ok()
            .expect("create translator");
        assert_eq!(translator.name(), "ExampleMnemonic");

        // Match padded binary for string value
        let (label_one, kind_one) =
            translator.basic_translate(4, &VariableValue::String("0001".into()));
        assert_eq!(label_one, "ONE");
        match kind_one {
            ValueKind::Custom(c) => assert!(color_eq(c, Color32::GREEN)),
            _ => panic!("expected custom green"),
        }

        // Integer value translation (BigUint) should pad and map to label TWO
        let (label_two, kind_two) =
            translator.basic_translate(4, &VariableValue::BigUint(BigUint::from(2u32)));
        assert_eq!(label_two, "TWO");
        match kind_two {
            ValueKind::Custom(c) => assert!(color_eq(c, Color32::BLUE)),
            _ => panic!("expected custom blue"),
        }

        // Fallback for value not in map (all-defined binary -> Normal)
        let (label_unknown, kind_unknown) =
            translator.basic_translate(4, &VariableValue::String("0011".into()));
        assert_eq!(label_unknown, "0011");
        assert!(matches!(kind_unknown, ValueKind::Normal));

        // Value with x should return Error
        let (label_undef, kind_undef) =
            translator.basic_translate(4, &VariableValue::String("00x1".into()));
        assert_eq!(label_undef, "00x1");
        assert!(matches!(kind_undef, ValueKind::Undef));

        // Value with u char should return Undef
        let (label_undef, kind_undef) =
            translator.basic_translate(4, &VariableValue::String("00u1".into()));
        assert_eq!(label_undef, "00u1");
        assert!(matches!(kind_undef, ValueKind::Undef));
    }

    #[test]
    fn filename_derived_name_when_no_name_line_present() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros();
        let stem = format!("auto_file_name_{}", ts);
        let mut path = std::env::temp_dir();
        path.push(format!("{stem}.mnemonic"));

        // No Name: line, first line is Bits: so default name should be file stem
        let content = "Bits: 3\n0 ZERO red\n1 ONE green\n2 TWO blue";
        std::fs::write(&path, content).expect("write mnemonic file");
        let translator = MnemonicTranslator::new_from_file(&path)
            .ok()
            .expect("create translator");
        assert_eq!(translator.name(), stem); // derived from filename
        assert_eq!(translator.map.bits, 3);
        // Translate a BigUint value 2 => binary 10 padded to 3 bits -> 010 maps to TWO
        let (label_two, kind_two) =
            translator.basic_translate(3, &VariableValue::BigUint(BigUint::from(2u32)));
        assert_eq!(label_two, "TWO");
        match kind_two {
            ValueKind::Custom(c) => assert!(color_eq(c, Color32::BLUE)),
            _ => panic!("expected custom blue"),
        }

        // Value not present (binary 111) should fallback to Normal
        let (fallback, vk_fb) = translator.basic_translate(3, &VariableValue::String("111".into()));
        assert_eq!(fallback, "111");
        assert!(matches!(vk_fb, ValueKind::Normal));

        // Value with z should be HighImp
        let (high_imp_val, high_imp_kind) =
            translator.basic_translate(3, &VariableValue::String("1z1".into()));
        assert_eq!(high_imp_val, "1z1");
        assert!(matches!(high_imp_kind, ValueKind::HighImp));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let content = "// top comment\n   // another comment\n\nName: Commented\n// mid\nBits: 3\n// entry comment\n0 ZERO red // inline A\n# Hash comment\n\n1 ONE green // inline B\n   // trailing inline comment line\n2 TWO blue // final";
        let map = parse_content_with_default_name(content, None).unwrap();
        assert_eq!(map.name, Some("Commented".to_string()));
        assert_eq!(map.bits, 3);
        assert_eq!(map.entries.len(), 3);
        assert_eq!(map.entries.get("000").unwrap().label, "ZERO");
        assert_eq!(map.entries.get("001").unwrap().label, "ONE");
        assert_eq!(map.entries.get("010").unwrap().label, "TWO");
    }

    #[test]
    fn parse_and_translate_valuekind_keywords() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!("mnemonic_kinds_{}.txt", ts));

        // Define entries with various ValueKind keywords
        let content = "Bits: 4\n0 ZERO warn\n1 ONE undef\n2 TWO highimp\n3 THREE dontcare\n4 FOUR weak\n5 FIVE error\n6 SIX normal";
        std::fs::write(&path, content).expect("write mnemonic file");

        let translator = MnemonicTranslator::new_from_file(&path)
            .ok()
            .expect("create translator");
        assert_eq!(translator.map.bits, 4);

        // Test each ValueKind keyword
        let (label_zero, kind_zero) =
            translator.basic_translate(4, &VariableValue::String("0000".into()));
        assert_eq!(label_zero, "ZERO");
        assert!(matches!(kind_zero, ValueKind::Warn));

        let (label_one, kind_one) =
            translator.basic_translate(4, &VariableValue::String("0001".into()));
        assert_eq!(label_one, "ONE");
        assert!(matches!(kind_one, ValueKind::Undef));

        let (label_two, kind_two) =
            translator.basic_translate(4, &VariableValue::String("0010".into()));
        assert_eq!(label_two, "TWO");
        assert!(matches!(kind_two, ValueKind::HighImp));

        let (label_three, kind_three) =
            translator.basic_translate(4, &VariableValue::String("0011".into()));
        assert_eq!(label_three, "THREE");
        assert!(matches!(kind_three, ValueKind::DontCare));

        let (label_four, kind_four) =
            translator.basic_translate(4, &VariableValue::String("0100".into()));
        assert_eq!(label_four, "FOUR");
        assert!(matches!(kind_four, ValueKind::Weak));

        let (label_five, kind_five) =
            translator.basic_translate(4, &VariableValue::String("0101".into()));
        assert_eq!(label_five, "FIVE");
        assert!(matches!(kind_five, ValueKind::Error));

        let (label_six, kind_six) =
            translator.basic_translate(4, &VariableValue::String("0110".into()));
        assert_eq!(label_six, "SIX");
        assert!(matches!(kind_six, ValueKind::Normal));
    }

    #[test]
    fn error_variants_coverage() {
        // InvalidBitsLineFormat
        match parse_bits_line("Bits") {
            Err(MnemonicParseError::InvalidBitsLineFormat) => {}
            other => panic!("expected InvalidBitsLineFormat, got: {:?}", other),
        }

        // InvalidBitsValue
        match parse_bits_line("Bits: notanumber") {
            Err(MnemonicParseError::InvalidBitsValue(v)) => assert_eq!(v, "notanumber"),
            other => panic!("expected InvalidBitsValue, got: {:?}", other),
        }

        // InvalidHex via parse_first_column
        match parse_first_column("0xZZ") {
            Err(MnemonicParseError::InvalidHex(tok)) => assert_eq!(tok, "0xZZ"),
            other => panic!("expected InvalidHex, got: {:?}", other),
        }

        // UnknownColor via parse_color_kind
        match parse_color_kind("not_a_color") {
            Err(MnemonicParseError::UnknownKindColor(s)) => assert_eq!(s, "not_a_color"),
            other => panic!("expected UnknownColor, got: {:?}", other),
        }

        // BinaryTooWide
        match normalize_first_column("1111", 3) {
            Err(MnemonicParseError::BinaryTooWide {
                value,
                required,
                specified,
            }) => {
                assert_eq!(value, "1111");
                assert!(required > specified);
            }
            other => panic!("expected BinaryTooWide, got: {:?}", other),
        }

        // StringLengthMismatch
        match normalize_first_column("ABCD", 3) {
            Err(MnemonicParseError::StringLengthMismatch {
                value,
                value_len,
                expected,
            }) => {
                assert_eq!(value, "ABCD");
                assert_eq!(value_len, 4);
                assert_eq!(expected, 3);
            }
            other => panic!("expected StringLengthMismatch, got: {:?}", other),
        }

        // MissingSecondColumn and EmptyLine via parse_line
        match parse_line("") {
            Err(MnemonicParseError::EmptyLine) => {}
            other => panic!("expected EmptyLine, got: {:?}", other),
        }

        match parse_line("0101") {
            Err(MnemonicParseError::MissingSecondColumn) => {}
            other => panic!("expected MissingSecondColumn, got: {:?}", other),
        }

        // LineError produced by parse_content_with_default_name when an entry line is invalid
        let content = "Bits: 4\n0 ZERO red\nBADLINE";
        match parse_content_with_default_name(content, None) {
            Err(MnemonicParseError::LineError {
                line,
                content,
                message: _,
            }) => {
                assert_eq!(line, 3);
                assert_eq!(content, "BADLINE");
            }
            other => panic!("expected LineError, got: {:?}", other),
        }

        // Io error via MnemonicMap::new with non-existent file
        let path = std::path::PathBuf::from("/this/path/should/not/exist/mnemonic.tmp");
        match MnemonicMap::new(path) {
            Err(MnemonicParseError::Io(_)) => {}
            other => panic!("expected Io, got: {:?}", other),
        }
    }
}
