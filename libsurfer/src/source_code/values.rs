//! Cursor values shown inline in the source tile.
//!
//! The tile joins a token to its elaborated symbol through the source index and to a
//! recorded signal through the trace binding; the waveform document then supplies
//! the value at the cursor. This module holds the parts of that path that are pure
//! text and number work: SystemVerilog constants from the VDB, packed struct fields
//! sliced by the elaborated type, index expressions after an identifier, and the
//! dotted name a token belongs to.

use crate::source_index::{Span, TokenClass};
use num::BigUint;

/// How a value is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueState {
    /// A value like any other.
    Normal,
    /// The signal's last transition is at the cursor.
    Changed,
    /// Undefined or high-impedance bits, or no sample before the cursor.
    Unknown,
    /// The symbol exists in the design but the recording has no signal for it.
    Missing,
}

/// One value on a line, anchored to the token it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shown {
    /// Byte range of the anchoring token on the line.
    pub start: u32,
    pub end: u32,
    /// Name as written in the source, with member and index suffixes.
    pub name: String,
    pub text: String,
    pub state: ValueState,
}

/// Values of one line in order of first appearance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineValues {
    pub shown: Vec<Shown>,
    /// False while a referenced signal is still loading, so the line is not cached.
    pub complete: bool,
}

/// Text of an elaborated constant such as `8'hc8`, `'sh8` or `32'd200`, shortened to the
/// digits in the radix the simulator wrote: `c8`, `8`, `200`.
pub(crate) fn constant_text(literal: &str) -> String {
    let literal = literal.trim();
    let Some((_, rest)) = literal.split_once('\'') else {
        return literal.to_owned();
    };
    let rest = rest.strip_prefix(['s', 'S']).unwrap_or(rest);
    let mut chars = rest.chars();
    let base = chars.next();
    let digits: String = chars.filter(|c| *c != '_').collect();
    match base {
        Some('h' | 'H' | 'b' | 'B' | 'o' | 'O') => {
            let trimmed = digits.trim_start_matches('0');
            if trimmed.is_empty() {
                "0".to_owned()
            } else {
                trimmed.to_lowercase()
            }
        }
        Some('d' | 'D') => digits,
        _ => literal.to_owned(),
    }
}

/// A field of a packed struct type, from the VDB's elaborated type text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Field {
    pub name: String,
    pub width: u32,
}

/// Fields of `struct{logic valid;logic[3:0] tag;}pkt_t` in declaration order, most
/// significant first. Nested aggregates and unknown types give `None`.
pub(crate) fn struct_fields(type_text: &str) -> Option<Vec<Field>> {
    let body = type_text.trim_start().strip_prefix("struct")?.trim_start();
    let body = body.strip_prefix("packed").map_or(body, str::trim_start);
    let body = body.strip_prefix('{')?;
    let end = body.find('}')?;
    let mut fields = Vec::new();
    for declaration in body[..end].split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        let (ty, name) = declaration.rsplit_once(|c: char| c.is_whitespace() || c == ']')?;
        let ty = declaration[..ty.len() + 1].trim();
        let name = name.trim();
        if name.is_empty() || name.contains('[') {
            return None;
        }
        fields.push(Field {
            name: name.to_owned(),
            width: packed_width(ty)?,
        });
    }
    (!fields.is_empty()).then_some(fields)
}

/// Bits of a packed scalar or vector type such as `logic`, `bit[7:0]` or `int`.
fn packed_width(ty: &str) -> Option<u32> {
    let ty = ty.trim();
    if let Some(open) = ty.find('[') {
        let close = ty[open..].find(']')? + open;
        let (hi, lo) = ty[open + 1..close].split_once(':')?;
        let hi: i64 = hi.trim().parse().ok()?;
        let lo: i64 = lo.trim().parse().ok()?;
        let base = ty[..open].trim();
        if !matches!(base, "logic" | "bit" | "reg" | "wire") {
            return None;
        }
        return u32::try_from((hi - lo).abs() + 1).ok();
    }
    match ty {
        "logic" | "bit" | "reg" | "wire" => Some(1),
        "byte" => Some(8),
        "shortint" => Some(16),
        "int" | "integer" => Some(32),
        "longint" => Some(64),
        _ => None,
    }
}

/// Hex digits of `bits` of a value, or the raw bit characters when the value
/// contains unknowns. `hi` is the most significant selected bit, inclusive.
pub(crate) fn slice_text(
    value: &surfer_translation_types::VariableValue,
    width: u32,
    hi: u32,
    lo: u32,
) -> (String, bool) {
    use surfer_translation_types::VariableValue;
    let bits = hi - lo + 1;
    match value {
        VariableValue::BigUint(v) => {
            let sliced = (v >> lo) & ((BigUint::from(1u8) << bits) - BigUint::from(1u8));
            (hex_text(&sliced, bits), false)
        }
        VariableValue::String(s) => {
            // Bit strings are written most significant first and may be shorter than
            // the declared width.
            let padded: String = std::iter::repeat_n('0', (width as usize).saturating_sub(s.len()))
                .chain(s.chars())
                .collect();
            let from = padded.len().saturating_sub(hi as usize + 1);
            let to = padded.len().saturating_sub(lo as usize);
            let text = padded[from..to].to_owned();
            let unknown = text.contains(['x', 'X', 'z', 'Z', '?']);
            if unknown {
                (text, true)
            } else {
                match BigUint::parse_bytes(text.as_bytes(), 2) {
                    Some(v) => (hex_text(&v, bits), false),
                    None => (text, true),
                }
            }
        }
    }
}

/// Zero-padded lowercase hex of a `bits`-wide value; single bits print as `0`/`1`.
pub(crate) fn hex_text(value: &BigUint, bits: u32) -> String {
    if bits == 1 {
        return format!("{value}");
    }
    format!("{:0width$x}", value, width = bits.div_ceil(4) as usize)
}

/// An index expression following a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Index {
    /// `[3]`
    Constant(u64),
    /// `[i]`, with the byte range of the identifier.
    Identifier(u32, u32),
    /// Anything else, like `[k+1]` or `[7:0]`.
    Dynamic,
}

/// Parses `[expr]` right after byte `end` of a line, returning the index and the byte
/// where it ends.
pub(crate) fn index_after(text: &str, end: u32) -> Option<(Index, u32)> {
    let rest = text.get(end as usize..)?;
    let after_bracket = rest.strip_prefix('[')?;
    let close = after_bracket.find(']')?;
    let inner = &after_bracket[..close];
    let end_byte = end + 1 + close as u32 + 1;
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(constant) = trimmed.parse::<u64>() {
        return Some((Index::Constant(constant), end_byte));
    }
    let is_identifier = trimmed
        .chars()
        .enumerate()
        .all(|(i, c)| c == '_' || c == '$' || (i > 0 && c.is_ascii_digit()) || c.is_alphabetic());
    if is_identifier {
        let offset = inner.len() - inner.trim_start().len();
        let start = end + 1 + offset as u32;
        return Some((
            Index::Identifier(start, start + trimmed.len() as u32),
            end_byte,
        ));
    }
    Some((Index::Dynamic, end_byte))
}

/// The dotted chain the token at `spans[at]` ends, like `bus.valid` for `valid` in
/// `bus.valid`, with the byte where the chain starts. Index brackets between the parts
/// are kept as written.
pub(crate) fn dotted_name(text: &str, spans: &[Span], at: usize) -> (u32, String) {
    let mut start = spans[at].start;
    let mut index = at;
    loop {
        let before = &text[..start as usize];
        let Some(rest) = before.strip_suffix('.') else {
            break;
        };
        let mut rest = rest;
        // Skip an index like `[0]` before the dot.
        if let Some(open) = rest.strip_suffix(']').and_then(|r| r.rfind('[')) {
            rest = &rest[..open];
        }
        // Operators are tokens too, so look for the symbol ending right before the dot.
        let previous = spans[..index]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, span)| span.end as usize <= rest.len());
        match previous {
            Some((i, span)) if span.end as usize == rest.len() && span.class.is_symbol() => {
                start = span.start;
                index = i;
            }
            _ => break,
        }
    }
    (
        start,
        text[start as usize..spans[at].end as usize].to_owned(),
    )
}

/// Whether a token contributes a value: variables, parameters, and members.
pub(crate) fn carries_value(span: &Span) -> bool {
    matches!(
        span.class,
        TokenClass::Variable | TokenClass::Parameter | TokenClass::Property
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::Modifiers;
    use surfer_translation_types::VariableValue;

    fn span(start: u32, end: u32, class: TokenClass) -> Span {
        Span {
            start,
            end,
            class,
            modifiers: Modifiers::default(),
            declaration: None,
        }
    }

    #[test]
    fn constants_keep_the_simulator_radix_without_padding() {
        assert_eq!(constant_text("8'hc8"), "c8");
        assert_eq!(constant_text("'sh8"), "8");
        assert_eq!(constant_text("1'h0"), "0");
        assert_eq!(constant_text("32'd200"), "200");
        assert_eq!(constant_text("4'b0011"), "11");
        assert_eq!(constant_text("3.5"), "3.5");
    }

    #[test]
    fn struct_fields_come_from_the_elaborated_type_text() {
        let fields = struct_fields("struct{logic valid;logic[3:0] tag;}feat_pkg::pkt_t").unwrap();
        assert_eq!(
            fields,
            vec![
                Field {
                    name: "valid".into(),
                    width: 1
                },
                Field {
                    name: "tag".into(),
                    width: 4
                }
            ]
        );
        assert_eq!(struct_fields("logic[7:0]"), None);
        assert_eq!(struct_fields("struct{struct{logic a;} b;}"), None);
        assert_eq!(
            struct_fields("struct packed{int a;byte b;}").unwrap().len(),
            2
        );
    }

    #[test]
    fn slices_take_hex_digits_or_raw_unknown_bits() {
        let value = VariableValue::BigUint(BigUint::from(0b10011u32));
        assert_eq!(slice_text(&value, 5, 4, 4), ("1".to_owned(), false));
        assert_eq!(slice_text(&value, 5, 3, 0), ("3".to_owned(), false));
        let unknown = VariableValue::String("1xx11".into());
        assert_eq!(slice_text(&unknown, 5, 3, 0), ("xx11".to_owned(), true));
        assert_eq!(slice_text(&unknown, 5, 4, 4), ("1".to_owned(), false));
        assert_eq!(hex_text(&BigUint::from(2u8), 8), "02");
    }

    #[test]
    fn index_expressions_after_a_token() {
        assert_eq!(index_after("a[3] + b", 1), Some((Index::Constant(3), 4)));
        assert_eq!(index_after("a[ i ]", 1), Some((Index::Identifier(3, 4), 6)));
        assert_eq!(index_after("a[k+1]", 1), Some((Index::Dynamic, 6)));
        assert_eq!(index_after("a + b", 1), None);
    }

    #[test]
    fn dotted_names_walk_back_over_members_and_indices() {
        let text = "x = bus.valid & lanes[0].u.count;";
        let spans = vec![
            span(0, 1, TokenClass::Variable),
            span(4, 7, TokenClass::Instance),
            span(7, 8, TokenClass::Operator),
            span(8, 13, TokenClass::Variable),
            span(16, 21, TokenClass::Instance),
            span(21, 22, TokenClass::Operator),
            span(24, 25, TokenClass::Operator),
            span(25, 26, TokenClass::Instance),
            span(26, 27, TokenClass::Operator),
            span(27, 32, TokenClass::Variable),
        ];
        assert_eq!(dotted_name(text, &spans, 3), (4, "bus.valid".into()));
        assert_eq!(
            dotted_name(text, &spans, 9),
            (16, "lanes[0].u.count".into())
        );
        assert_eq!(dotted_name(text, &spans, 0), (0, "x".into()));
    }
}
