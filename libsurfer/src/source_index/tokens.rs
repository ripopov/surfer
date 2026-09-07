//! Token classes of the VDB source index, grouped per line for rendering.
//!
//! The index names its classes and modifier bits in a legend; Surfer resolves that
//! legend once per design so unknown entries degrade to plain text instead of
//! breaking the tile.

use std::collections::HashMap;
use vtr_vdb::{IndexLocation, IndexedFile};

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
    fn from_name(name: &str) -> Self {
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
            "module" => Self::Module,
            "interface" => Self::Interface,
            "package" => Self::Package,
            "instance" => Self::Instance,
            "function" => Self::Function,
            "property" => Self::Property,
            "namespace" => Self::Namespace,
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

    /// Lower-case name for tooltips.
    pub fn label(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Comment => "comment",
            Self::Number => "number",
            Self::String => "string",
            Self::Operator => "operator",
            Self::Macro => "macro",
            Self::Variable => "variable",
            Self::Parameter => "parameter",
            Self::EnumMember => "enum member",
            Self::Type => "type",
            Self::Module => "module",
            Self::Interface => "interface",
            Self::Package => "package",
            Self::Instance => "instance",
            Self::Function => "function",
            Self::Property => "member",
            Self::Namespace => "scope",
            Self::Other => "token",
        }
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

    fn from_name(name: &str) -> Self {
        match name {
            "declaration" => Self::DECLARATION,
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

/// The legend of one index, resolved to Surfer's classes and modifier bits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Legend {
    classes: Vec<TokenClass>,
    modifiers: Vec<Modifiers>,
}

impl Legend {
    pub fn new(classes: &[String], modifiers: &[String]) -> Self {
        Self {
            classes: classes.iter().map(|c| TokenClass::from_name(c)).collect(),
            modifiers: modifiers.iter().map(|m| Modifiers::from_name(m)).collect(),
        }
    }

    fn class(&self, number: u32) -> TokenClass {
        self.classes
            .get(number as usize)
            .copied()
            .unwrap_or(TokenClass::Other)
    }

    fn modifiers(&self, bits: u32) -> Modifiers {
        self.modifiers
            .iter()
            .enumerate()
            .filter(|(bit, _)| bits & (1 << bit) != 0)
            .fold(Modifiers::default(), |acc, (_, m)| acc.union(*m))
    }
}

/// One classified span on a line, in byte offsets of that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub class: TokenClass,
    pub modifiers: Modifiers,
    /// Declaration of the symbol the token denotes, in index coordinates.
    pub declaration: Option<IndexLocation>,
}

/// Tokens of one file grouped by zero-based line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileTokens {
    lines: HashMap<u32, Vec<Span>>,
    count: usize,
}

impl FileTokens {
    pub fn new(file: &IndexedFile, legend: &Legend) -> Self {
        let mut lines: HashMap<u32, Vec<Span>> = HashMap::new();
        let mut count = 0;
        for token in file.tokens() {
            if token.line == 0 || token.column == 0 {
                continue;
            }
            let start = token.column - 1;
            lines.entry(token.line - 1).or_default().push(Span {
                start,
                end: start + token.length,
                class: legend.class(token.class),
                modifiers: legend.modifiers(token.modifiers),
                declaration: token.declaration,
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

    /// Number of classified tokens in the file.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Every line with tokens, in no particular order.
    pub fn lines(&self) -> impl Iterator<Item = (u32, &[Span])> {
        self.lines
            .iter()
            .map(|(line, spans)| (*line, spans.as_slice()))
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
    fn index_positions_become_zero_based_line_spans() {
        let file: IndexedFile = serde_json::from_value(serde_json::json!({
            "path": "a.sv",
            // `module` on line 1, `clk` (input|clock) at column 21 of line 2, a comment
            // at column 5 of line 4.
            "tokens": [1, 1, 6, 0, 0, 2, 21, 3, 2, 0b100010, 4, 5, 10, 1, 0],
            "declarations": [1, 0, 2, 21]
        }))
        .unwrap();
        let tokens = FileTokens::new(&file, &legend());
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens.line(0),
            &[Span {
                start: 0,
                end: 6,
                class: TokenClass::Keyword,
                modifiers: Modifiers::default(),
                declaration: None,
            }]
        );
        let clk = &tokens.line(1)[0];
        assert_eq!(clk.class, TokenClass::Variable);
        assert!(clk.modifiers.contains(Modifiers::INPUT));
        assert!(clk.modifiers.contains(Modifiers::CLOCK));
        assert!(!clk.modifiers.contains(Modifiers::OUTPUT));
        assert_eq!(
            clk.declaration,
            Some(IndexLocation {
                file: 0,
                line: 2,
                column: 21
            })
        );
        assert_eq!(tokens.line(1).len(), 1);
        assert_eq!(tokens.line(3)[0].class, TokenClass::Comment);
        assert!(tokens.line(2).is_empty());
    }

    #[test]
    fn unknown_legend_entries_are_tolerated() {
        let legend = Legend::new(&["weird".to_string()], &["odd".to_string()]);
        let file: IndexedFile = serde_json::from_value(serde_json::json!({
            "path": "a.sv", "tokens": [1, 1, 1, 0, 1, 1, 3, 1, 9, 0]
        }))
        .unwrap();
        let tokens = FileTokens::new(&file, &legend);
        assert_eq!(tokens.line(0)[0].class, TokenClass::Other);
        assert_eq!(tokens.line(0)[0].modifiers, Modifiers::default());
        assert_eq!(tokens.line(0)[1].class, TokenClass::Other);
        assert_eq!(tokens.len(), 2);
    }
}
