use ecolor::Color32;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use surfer_translation_types::{
    extend_string, BasicTranslator, TranslationPreference, ValueKind, VariableValue,
};

use crate::{
    translation::{check_single_wordlength, kind_for_binary_representation},
    wave_container::{ScopeId, VarId, VariableMeta},
};

#[derive(Debug, Clone, PartialEq)]
pub struct MnemonicEntry {
    pub label: String,
    pub color: Option<Color32>,
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

impl MnemonicTranslator {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        MnemonicTranslator {
            map: MnemonicMap::new(path),
        }
    }
}

impl BasicTranslator<VarId, ScopeId> for MnemonicTranslator {
    fn name(&self) -> String {
        self.map.name.clone().unwrap_or_default()
    }

    fn basic_translate(&self, num_bits: u64, value: &VariableValue) -> (String, ValueKind) {
        let var_string = match value {
            VariableValue::BigUint(v) => format!("{v:0width$b}", width = num_bits as usize),
            VariableValue::String(s) => {
                format!("{extra_bits}{s}", extra_bits = extend_string(s, num_bits))
            }
        };

        if let Some(entry) = self.map.entries.get(&var_string) {
            if let Some(color) = &entry.color {
                (entry.label.clone(), ValueKind::Custom(*color))
            } else {
                (entry.label.clone(), ValueKind::Normal)
            }
        } else {
            let val_kind = kind_for_binary_representation(&var_string);
            (var_string, val_kind)
        }
    }

    fn translates(&self, variable: &VariableMeta) -> eyre::Result<TranslationPreference> {
        check_single_wordlength(variable.num_bits, self.map.bits as u32)
    }

    fn variable_info(
        &self,
        _variable: &surfer_translation_types::VariableMeta<VarId, ScopeId>,
    ) -> eyre::Result<surfer_translation_types::VariableInfo> {
        Ok(surfer_translation_types::VariableInfo::Bits)
    }
}

impl MnemonicMap {
    pub fn new<P: AsRef<Path>>(file: P) -> Self {
        parse_file(file).unwrap()
    }
}

pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<MnemonicMap, String> {
    let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;

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
) -> Result<MnemonicMap, String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Ok(MnemonicMap {
            name: default_name,
            bits: 0,
            entries: HashMap::new(),
        });
    }

    let mut line_idx = 0;
    let mut name = None;
    let mut bits = None;

    // Skip leading blank/comment lines
    while line_idx < lines.len() {
        let trimmed = lines[line_idx].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            line_idx += 1;
            continue;
        }
        break;
    }

    // Check first non-comment line for Name: or heuristic name
    if line_idx < lines.len() {
        let first_line = lines[line_idx].trim();
        if !first_line.starts_with('#') {
            if first_line.to_lowercase().starts_with("name:") {
                name = Some(first_line[5..].trim().to_string());
                line_idx += 1;
            } else if looks_like_name(first_line) {
                name = Some(first_line.trim().to_string());
                line_idx += 1;
            }
        }
    }

    // Skip intervening blank/comment lines before Bits:
    while line_idx < lines.len() {
        let trimmed = lines[line_idx].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            line_idx += 1;
            continue;
        }
        break;
    }

    // Check for Bits: line (could be first meaningful or second meaningful line)
    if line_idx < lines.len() {
        let current_line = lines[line_idx].trim();
        if current_line.to_lowercase().starts_with("bits:") {
            bits = Some(parse_bits_line(current_line)?);
            line_idx += 1;
        }
    }

    // Parse remaining lines as entries (first pass to get raw data)
    let mut raw_entries = Vec::new();
    for (idx, line) in lines.iter().enumerate().skip(line_idx) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Ok(entry) => raw_entries.push(entry),
            Err(e) => return Err(format!("Line {}: {}", idx + 1, e)),
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
        entries.insert(
            normalized_first,
            MnemonicEntry {
                label: second,
                color,
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

fn looks_like_name(line: &str) -> bool {
    // Heuristic: if the line doesn't start with a number or hex prefix,
    // and doesn't contain multiple space-separated tokens (or has quoted strings),
    // it might be a name
    let trimmed = line.trim();

    // If it starts with "bits:", it's not a name
    if trimmed.to_lowercase().starts_with("bits:") {
        return false;
    }

    // If it starts with 0x or a digit, probably not a name
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        return false;
    }
    if trimmed.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return false;
    }

    // If it has very few tokens (not quoted), might be a name
    let tokens = split_tokens(trimmed);
    tokens.len() <= 2 && !trimmed.contains('"')
}

fn parse_bits_line(line: &str) -> Result<u32, String> {
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err("Invalid Bits: line format".to_string());
    }

    let bits_str = parts[1].trim();
    bits_str
        .parse::<u32>()
        .map_err(|_| format!("Invalid bits value: {}", bits_str))
}

fn normalize_first_column(first: &str, bit_width: usize) -> Result<String, String> {
    // Check if the first column contains only '0' and '1' (binary string)
    let is_binary = first.chars().all(|c| c == '0' || c == '1');

    if is_binary {
        // Pad binary string to match bit_width
        let padded = pad_binary_string(first, bit_width);
        if padded.len() > bit_width {
            return Err(format!(
                "Binary string '{}' requires {} bits, but only {} bits specified",
                first,
                padded.len(),
                bit_width
            ));
        }
        Ok(padded)
    } else {
        // It's a regular string, validate it matches bit_width
        if first.len() != bit_width {
            return Err(format!(
                "String '{}' has {} characters, expected {} characters to match bit width",
                first,
                first.len(),
                bit_width
            ));
        }
        Ok(first.to_string())
    }
}

fn parse_line(line: &str) -> Result<(String, String, Option<Color32>), String> {
    let tokens = split_tokens(line);

    if tokens.is_empty() {
        return Err("Empty line".to_string());
    }

    let first = parse_first_column(&tokens[0])?;

    if tokens.len() < 2 {
        return Err("Missing second column".to_string());
    }

    let second = tokens[1].clone();

    let color = if tokens.len() >= 3 {
        Some(parse_color(&tokens[2])?)
    } else {
        None
    };

    Ok((first, second, color))
}

fn parse_first_column(token: &str) -> Result<String, String> {
    // Try hex number (0x prefix)
    if token.starts_with("0x") || token.starts_with("0X") {
        let hex_str = &token[2..];
        let num = u64::from_str_radix(hex_str, 16)
            .map_err(|_| format!("Invalid hex number: {}", token))?;
        // Return binary string without padding (will be padded later)
        return Ok(format!("{:b}", num));
    }

    // Check if it's a number containing only 0s and 1s - treat as binary
    let all_binary_digits = token.chars().all(|c| c == '0' || c == '1');
    if all_binary_digits && !token.is_empty() {
        // It's already a binary string, return as-is
        return Ok(token.to_string());
    }

    // Try decimal number (with digits other than 0 and 1)
    if let Ok(num) = token.parse::<u64>() {
        // Return binary string without padding (will be padded later)
        return Ok(format!("{:b}", num));
    }

    // Otherwise, treat as string
    Ok(token.to_string())
}

fn pad_binary_string(binary: &str, width: usize) -> String {
    format!("{:0>width$}", binary, width = width)
}

fn parse_color(token: &str) -> Result<Color32, String> {
    // Try hex color (#RRGGBB or RRGGBB)
    let hex_str = token.strip_prefix('#').unwrap_or(token);

    if hex_str.len() == 6 && hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        let r = u8::from_str_radix(&hex_str[0..2], 16).unwrap();
        let g = u8::from_str_radix(&hex_str[2..4], 16).unwrap();
        let b = u8::from_str_radix(&hex_str[4..6], 16).unwrap();
        return Ok(Color32::from_rgb(r, g, b));
    }

    // Try named colors
    match token.to_lowercase().as_str() {
        "black" => Ok(Color32::BLACK),
        "white" => Ok(Color32::WHITE),
        "red" => Ok(Color32::RED),
        "green" => Ok(Color32::GREEN),
        "blue" => Ok(Color32::BLUE),
        "yellow" => Ok(Color32::YELLOW),
        "cyan" => Ok(Color32::from_rgb(0, 255, 255)),
        "magenta" => Ok(Color32::from_rgb(255, 0, 255)),
        "gray" | "grey" => Ok(Color32::GRAY),
        "light_gray" | "light_grey" => Ok(Color32::LIGHT_GRAY),
        "dark_gray" | "dark_grey" => Ok(Color32::DARK_GRAY),
        _ => Err(format!("Unknown color: {}", token)),
    }
}

fn split_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
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
        assert!(zero.color.is_some());
        assert!(color_eq(zero.color.unwrap(), Color32::RED));
        let one = map.entries.get("0001").expect("ONE entry");
        assert_eq!(one.label, "ONE");
        assert!(one.color.is_some());
        // #00FF00 => green
        let green = one.color.unwrap();
        assert!(color_eq(green, Color32::from_rgb(0, 255, 0)));
        let ten = map.entries.get("1010").expect("TEN entry");
        assert_eq!(ten.label, "TEN");
        assert!(ten.color.is_some());
        assert!(color_eq(ten.color.unwrap(), Color32::BLUE));
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
        let content = "Bits: 3\nABCD LABEL";
        let err = parse_content_with_default_name(content, None).unwrap_err();
        assert!(err.contains("expected 3"));
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
        use surfer_translation_types::ValueKind as VK;
        // Unique temp file path
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!("mnemonic_test_{}.txt", ts));

        let content = "Name: ExampleMnemonic\nBits: 4\n0 ZERO red\n1 ONE green\n10 TWO blue";
        std::fs::write(&path, content).expect("write mnemonic file");

        let translator = MnemonicTranslator::new(&path);
        assert_eq!(translator.name(), "ExampleMnemonic");

        // Match padded binary for string value
        let (label_one, kind_one) =
            translator.basic_translate(4, &VariableValue::String("0001".into()));
        assert_eq!(label_one, "ONE");
        match kind_one {
            VK::Custom(c) => assert!(color_eq(c, Color32::GREEN)),
            _ => panic!("expected custom green"),
        }

        // Integer value translation (BigUint) should pad and map to label TWO
        let (label_two, kind_two) =
            translator.basic_translate(4, &VariableValue::BigUint(BigUint::from(2u32)));
        assert_eq!(label_two, "TWO");
        match kind_two {
            VK::Custom(c) => assert!(color_eq(c, Color32::BLUE)),
            _ => panic!("expected custom blue"),
        }

        // Fallback for value not in map
        let (label_unknown, kind_unknown) =
            translator.basic_translate(4, &VariableValue::String("0011".into()));
        assert_eq!(label_unknown, "0011");
        assert!(matches!(kind_unknown, VK::Normal));
    }

    #[test]
    fn filename_derived_name_when_no_name_line_present() {
        use std::time::{SystemTime, UNIX_EPOCH};
        use surfer_translation_types::ValueKind as VK;
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
        let translator = MnemonicTranslator::new(&path);
        assert_eq!(translator.name(), stem); // derived from filename
        assert_eq!(translator.map.bits, 3);
        // Translate a BigUint value 2 => binary 10 padded to 3 bits -> 010 maps to TWO
        let (label_two, kind_two) =
            translator.basic_translate(3, &VariableValue::BigUint(BigUint::from(2u32)));
        assert_eq!(label_two, "TWO");
        match kind_two {
            VK::Custom(c) => assert!(color_eq(c, Color32::BLUE)),
            _ => panic!("expected custom blue"),
        }

        // Value not present (binary 111) should fallback
        let (fallback, vk_fb) = translator.basic_translate(3, &VariableValue::String("111".into()));
        assert_eq!(fallback, "111");
        assert!(matches!(vk_fb, VK::Normal));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let content = "# top comment\n   # another comment\n\nName: Commented\n# mid\nBits: 3\n# entry comment\n0 ZERO red\n\n1 ONE green\n   # trailing inline comment line\n2 TWO blue\n# final";
        let map = parse_content_with_default_name(content, None).unwrap();
        assert_eq!(map.name, Some("Commented".to_string()));
        assert_eq!(map.bits, 3);
        assert_eq!(map.entries.len(), 3);
        assert_eq!(map.entries.get("000").unwrap().label, "ZERO");
        assert_eq!(map.entries.get("001").unwrap().label, "ONE");
        assert_eq!(map.entries.get("010").unwrap().label, "TWO");
    }
}
