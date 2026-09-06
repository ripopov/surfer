//! Semantic token decoding and per-line indexing.
//!
//! The server returns the LSP relative encoding (five integers per token) against a legend
//! it advertised at initialization. Surfer resolves the legend once, then keeps tokens
//! grouped by line so the source tile can build one layout job per visible line.

use serde_json::Value;
use std::collections::HashMap;

/// Token classes the source tile can color. Unknown legend entries map to `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenClass {
    Keyword,
    Comment,
    Number,
    String,
    Operator,
    Macro,
    Variable,
    Parameter,
    EnumMember,
    Type,
    Module,
    Interface,
    Package,
    Instance,
    Function,
    Property,
    Namespace,
    Other,
}

impl TokenClass {
    fn from_legend(name: &str) -> Self {
        match name {
            "keyword" => Self::Keyword,
            "comment" => Self::Comment,
            "number" => Self::Number,
            "string" => Self::String,
            "operator" => Self::Operator,
            "macro" => Self::Macro,
            "variable" => Self::Variable,
            "parameter" => Self::Parameter,
            "enumMember" => Self::EnumMember,
            "type" => Self::Type,
            "module" | "class" => Self::Module,
            "interface" => Self::Interface,
            "package" | "namespace" => {
                if name == "package" {
                    Self::Package
                } else {
                    Self::Namespace
                }
            }
            "instance" => Self::Instance,
            "function" | "method" => Self::Function,
            "property" => Self::Property,
            _ => Self::Other,
        }
    }

    /// Whether a token of this class names something the design database may know.
    pub fn is_symbol(self) -> bool {
        matches!(
            self,
            Self::Variable
                | Self::Parameter
                | Self::EnumMember
                | Self::Instance
                | Self::Property
                | Self::Module
                | Self::Interface
                | Self::Function
                | Self::Type
                | Self::Package
        )
    }
}

/// Modifier bits as Surfer interprets them, independent of the legend order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u32);

impl Modifiers {
    pub const DECLARATION: Self = Self(1 << 0);
    pub const INPUT: Self = Self(1 << 1);
    pub const OUTPUT: Self = Self(1 << 2);
    pub const INOUT: Self = Self(1 << 3);
    pub const REF: Self = Self(1 << 4);
    pub const CLOCK: Self = Self(1 << 5);
    pub const READONLY: Self = Self(1 << 6);
    pub const DEFAULT_LIBRARY: Self = Self(1 << 7);
    pub const ARGUMENT: Self = Self(1 << 8);

    fn from_legend(name: &str) -> Self {
        match name {
            "declaration" | "definition" => Self::DECLARATION,
            "input" => Self::INPUT,
            "output" => Self::OUTPUT,
            "inout" => Self::INOUT,
            "ref" => Self::REF,
            "clock" => Self::CLOCK,
            "readonly" => Self::READONLY,
            "defaultLibrary" => Self::DEFAULT_LIBRARY,
            "argument" => Self::ARGUMENT,
            _ => Self(0),
        }
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The legend the server advertised, resolved to Surfer's classes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Legend {
    types: Vec<TokenClass>,
    modifiers: Vec<Modifiers>,
}

impl Legend {
    /// Reads `capabilities.semanticTokensProvider.legend` from an initialize result.
    pub fn from_capabilities(capabilities: &Value) -> Option<Self> {
        let legend = capabilities.get("semanticTokensProvider")?.get("legend")?;
        let names = |key: &str| -> Vec<String> {
            legend
                .get(key)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self::new(&names("tokenTypes"), &names("tokenModifiers")))
    }

    pub fn new(types: &[String], modifiers: &[String]) -> Self {
        Self {
            types: types.iter().map(|t| TokenClass::from_legend(t)).collect(),
            modifiers: modifiers
                .iter()
                .map(|m| Modifiers::from_legend(m))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

/// One classified span on a line, in byte offsets of that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub class: TokenClass,
    pub modifiers: Modifiers,
}

/// Tokens of one document grouped by zero-based line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineTokens {
    lines: HashMap<u32, Vec<Span>>,
    count: usize,
}

impl LineTokens {
    /// Decodes the relative encoding; malformed trailing data is ignored.
    pub fn decode(data: &[u32], legend: &Legend) -> Self {
        let mut lines: HashMap<u32, Vec<Span>> = HashMap::new();
        let mut line = 0u32;
        let mut character = 0u32;
        let mut count = 0;
        for chunk in data.chunks_exact(5) {
            let [delta_line, delta_start, length, kind, modifier_bits] =
                [chunk[0], chunk[1], chunk[2], chunk[3], chunk[4]];
            line += delta_line;
            character = if delta_line == 0 {
                character + delta_start
            } else {
                delta_start
            };
            let class = legend
                .types
                .get(kind as usize)
                .copied()
                .unwrap_or(TokenClass::Other);
            let mut modifiers = Modifiers::default();
            for (bit, modifier) in legend.modifiers.iter().enumerate() {
                if modifier_bits & (1 << bit) != 0 {
                    modifiers = modifiers.union(*modifier);
                }
            }
            lines.entry(line).or_default().push(Span {
                start: character,
                end: character + length,
                class,
                modifiers,
            });
            count += 1;
        }
        for spans in lines.values_mut() {
            spans.sort_by_key(|span| span.start);
        }
        Self { lines, count }
    }

    pub fn line(&self, line: u32) -> &[Span] {
        self.lines.get(&line).map_or(&[], Vec::as_slice)
    }

    /// The span covering byte `offset` of `line`, if any.
    pub fn at(&self, line: u32, offset: u32) -> Option<&Span> {
        self.line(line)
            .iter()
            .find(|span| span.start <= offset && offset < span.end)
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// A zero-based line/column range from the server (columns are byte offsets).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Range {
    /// Whether the span `[start, end)` on `line` lies inside this range.
    pub fn covers(&self, line: u32, start: u32, end: u32) -> bool {
        let from = Position {
            line,
            character: start,
        };
        let to = Position {
            line,
            character: end.max(start + 1) - 1,
        };
        from >= self.start && to < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legend() -> Legend {
        Legend::new(
            &["keyword", "comment", "variable", "parameter"].map(String::from),
            &["declaration", "input", "output", "inout", "ref", "clock"].map(String::from),
        )
    }

    #[test]
    fn decodes_relative_positions_and_modifiers() {
        // line 0: `module` keyword at 0, `clk` variable(input|clock) at 20 on line 1, then
        // a comment at column 4 of line 3.
        let data = [0, 0, 6, 0, 0, 1, 20, 3, 2, 0b100010, 2, 4, 10, 1, 0];
        let tokens = LineTokens::decode(&data, &legend());
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens.line(0),
            &[Span {
                start: 0,
                end: 6,
                class: TokenClass::Keyword,
                modifiers: Modifiers::default()
            }]
        );
        let clk = tokens.at(1, 21).unwrap();
        assert_eq!(clk.class, TokenClass::Variable);
        assert!(clk.modifiers.contains(Modifiers::INPUT));
        assert!(clk.modifiers.contains(Modifiers::CLOCK));
        assert!(!clk.modifiers.contains(Modifiers::OUTPUT));
        assert!(tokens.at(1, 23).is_none());
        assert_eq!(tokens.line(3)[0].class, TokenClass::Comment);
        assert!(tokens.line(2).is_empty());
    }

    #[test]
    fn unknown_legend_entries_are_tolerated() {
        let legend = Legend::new(&["weird".to_string()], &["odd".to_string()]);
        let tokens = LineTokens::decode(&[0, 0, 1, 0, 1, 0, 0, 1, 9, 0], &legend);
        assert_eq!(tokens.line(0)[0].class, TokenClass::Other);
        assert_eq!(tokens.line(0)[0].modifiers, Modifiers::default());
        assert_eq!(tokens.line(0)[1].class, TokenClass::Other);
    }

    #[test]
    fn legend_reads_capabilities() {
        let capabilities = serde_json::json!({
            "semanticTokensProvider": {"legend": {"tokenTypes": ["comment"], "tokenModifiers": []}}
        });
        let legend = Legend::from_capabilities(&capabilities).unwrap();
        assert_eq!(legend.types, vec![TokenClass::Comment]);
        assert!(Legend::from_capabilities(&serde_json::json!({})).is_none());
    }

    #[test]
    fn ranges_cover_spans_inclusively_on_start_and_exclusively_on_end() {
        let range = Range {
            start: Position {
                line: 2,
                character: 4,
            },
            end: Position {
                line: 5,
                character: 3,
            },
        };
        assert!(range.covers(2, 4, 10));
        assert!(!range.covers(2, 3, 10));
        assert!(range.covers(3, 0, 80));
        assert!(range.covers(5, 0, 3));
        assert!(!range.covers(5, 0, 4));
        assert!(!range.covers(6, 0, 1));
    }
}
