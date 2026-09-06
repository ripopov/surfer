//! Source-code tile and the small SystemVerilog lexer used by it.

use camino::Utf8PathBuf;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText, TextWrapMode, Ui};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use crate::source_index::SourceLocation;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCodeTile {
    pub file: Option<Utf8PathBuf>,
    pub line: u32,
    pub column: u32,
    #[serde(skip)]
    document: RefCell<Option<SourceDocument>>,
    #[serde(skip)]
    last_target: Cell<Option<(u32, u32)>>,
}

#[derive(Clone)]
struct SourceDocument {
    file: Utf8PathBuf,
    text: Result<Arc<str>, String>,
}

impl SourceCodeTile {
    pub(crate) fn open(&mut self, location: SourceLocation) {
        self.file = Some(location.file);
        self.line = location.line.max(1);
        self.column = location.column;
        self.last_target.set(None);
        *self.document.borrow_mut() = None;
    }

    pub(crate) fn ui(&self, ui: &mut Ui) {
        let Some(file) = &self.file else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a signal with VDB source information to open its source.");
            });
            return;
        };
        if self
            .document
            .borrow()
            .as_ref()
            .is_none_or(|document| document.file != *file)
        {
            *self.document.borrow_mut() = Some(SourceDocument {
                file: file.clone(),
                text: std::fs::read_to_string(file)
                    .map(Arc::from)
                    .map_err(|error| error.to_string()),
            });
        }
        let document = self.document.borrow();
        let contents = match &document.as_ref().expect("source document loaded").text {
            Ok(contents) => contents,
            Err(error) => {
                ui.colored_label(Color32::RED, format!("Unable to read {file}: {error}"));
                return;
            }
        };
        let scroll_to_target = self.last_target.replace(Some((self.line, self.column)))
            != Some((self.line, self.column));
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(file.file_name().unwrap_or(file.as_str()))
                    .strong()
                    .monospace(),
            )
            .on_hover_text(file.as_str());
            if self.line > 0 {
                ui.label(format!(":{}:{}", self.line, self.column));
            }
        });
        ui.separator();
        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
        let target = self.line.saturating_sub(1) as usize;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut in_block_comment = false;
                for (index, text) in contents.lines().enumerate() {
                    let selected = index == target;
                    let background = if selected {
                        ui.visuals().selection.bg_fill
                    } else {
                        Color32::TRANSPARENT
                    };
                    let mut row = LayoutJob::default();
                    row.append(
                        &format!("{:>5}  ", index + 1),
                        0.0,
                        TextFormat {
                            font_id: FontId::monospace(13.0),
                            color: ui.visuals().weak_text_color(),
                            ..Default::default()
                        },
                    );
                    append_systemverilog_line(
                        &mut row,
                        text,
                        ui.visuals().text_color(),
                        &mut in_block_comment,
                    );
                    let response = egui::Frame::NONE
                        .fill(background)
                        .show(ui, |ui| ui.add(egui::Label::new(row).selectable(true)))
                        .response;
                    if selected && scroll_to_target {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
    }
}

#[cfg(test)]
fn append_systemverilog(job: &mut LayoutJob, text: &str, normal: Color32) {
    let mut in_block_comment = false;
    append_systemverilog_line(job, text, normal, &mut in_block_comment);
}

fn append_systemverilog_line(
    job: &mut LayoutJob,
    text: &str,
    normal: Color32,
    in_block_comment: &mut bool,
) {
    const KEYWORDS: &[&str] = &[
        "accept_on",
        "always",
        "always_comb",
        "always_ff",
        "always_latch",
        "alias",
        "and",
        "assign",
        "assert",
        "assume",
        "automatic",
        "before",
        "begin",
        "bind",
        "bins",
        "binsof",
        "bit",
        "break",
        "byte",
        "case",
        "casex",
        "casez",
        "checker",
        "chandle",
        "class",
        "const",
        "constraint",
        "context",
        "continue",
        "covergroup",
        "coverpoint",
        "default",
        "dist",
        "do",
        "edge",
        "else",
        "end",
        "endchecker",
        "endcase",
        "endclass",
        "endclocking",
        "endgroup",
        "endfunction",
        "endinterface",
        "endmodule",
        "endpackage",
        "endtask",
        "endprogram",
        "endproperty",
        "endsequence",
        "endtable",
        "enum",
        "eventually",
        "expect",
        "export",
        "extends",
        "extern",
        "final",
        "first_match",
        "for",
        "foreach",
        "forkjoin",
        "function",
        "generate",
        "if",
        "iff",
        "ignore_bins",
        "illegal_bins",
        "implements",
        "import",
        "incdir",
        "include",
        "interface",
        "inside",
        "int",
        "integer",
        "interconnect",
        "let",
        "local",
        "localparam",
        "logic",
        "longint",
        "matches",
        "module",
        "modport",
        "nand",
        "new",
        "nexttime",
        "negedge",
        "null",
        "or",
        "package",
        "parameter",
        "posedge",
        "priority",
        "program",
        "protected",
        "property",
        "pure",
        "rand",
        "randc",
        "randcase",
        "randsequence",
        "ref",
        "reg",
        "return",
        "sequence",
        "shortint",
        "shortreal",
        "solve",
        "static",
        "string",
        "struct",
        "super",
        "task",
        "this",
        "throughout",
        "timeprecision",
        "timeunit",
        "type",
        "typedef",
        "union",
        "until",
        "until_with",
        "untyped",
        "var",
        "unique",
        "virtual",
        "void",
        "wait_order",
        "weak",
        "wire",
        "with",
        "within",
        "wildcard",
        "xnor",
        "xor",
    ];
    let keyword = Color32::from_rgb(190, 145, 255);
    let number = Color32::from_rgb(235, 175, 105);
    let comment = Color32::from_rgb(120, 155, 125);
    let string = Color32::from_rgb(225, 180, 110);
    let punctuation = Color32::from_rgb(120, 185, 225);
    let directive = Color32::from_rgb(100, 200, 210);
    let mut start = 0;
    let bytes = text.as_bytes();
    while start < bytes.len() {
        if *in_block_comment {
            let (end, closed) = text[start..]
                .find("*/")
                .map_or((bytes.len(), false), |end| (start + end + 2, true));
            append_token(job, &text[start..end], comment);
            *in_block_comment = !closed;
            start = end;
            continue;
        }
        if bytes[start..].starts_with(b"//") {
            append_token(job, &text[start..], comment);
            break;
        }
        if bytes[start..].starts_with(b"/*") {
            let (end, closed) = text[start + 2..]
                .find("*/")
                .map_or((bytes.len(), false), |end| (start + 2 + end + 2, true));
            append_token(job, &text[start..end], comment);
            *in_block_comment = !closed;
            start = end;
            continue;
        }
        let end = if bytes[start] == b'"' {
            let mut end = start + 1;
            while end < bytes.len() {
                if bytes[end] == b'"' && bytes[end - 1] != b'\\' {
                    end += 1;
                    break;
                }
                end += 1;
            }
            end
        } else if bytes[start] == b'`' {
            let mut end = start + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            end
        } else if bytes[start] == b'\\' {
            let mut end = start + 1;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            end
        } else if bytes[start].is_ascii_alphanumeric()
            || bytes[start] == b'_'
            || bytes[start] == b'\''
        {
            let mut end = start + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'\''))
            {
                end += 1;
            }
            end
        } else if bytes[start].is_ascii_whitespace() {
            let mut end = start + 1;
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            end
        } else {
            start + 1
        };
        let token = &text[start..end];
        let color = if token.starts_with('"') {
            string
        } else if token.starts_with('`') {
            directive
        } else if KEYWORDS.contains(&token) {
            keyword
        } else if token.as_bytes().first().is_some_and(u8::is_ascii_digit) || token.starts_with("'")
        {
            number
        } else if token.chars().all(|c| c.is_ascii_punctuation()) && !token.is_empty() {
            punctuation
        } else {
            normal
        };
        append_token(job, token, color);
        start = end;
    }
}

fn append_token(job: &mut LayoutJob, text: &str, color: Color32) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::monospace(13.0),
            color,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemverilog_lexer_preserves_text_and_marks_tokens() {
        let input = "always_ff @(posedge clk) q <= 8'h2a; // hold";
        let mut job = LayoutJob::default();
        append_systemverilog(&mut job, input, Color32::WHITE);
        assert_eq!(job.text, input);
        assert!(
            job.sections
                .iter()
                .any(|s| s.format.color == Color32::from_rgb(190, 145, 255))
        );
        assert!(
            job.sections
                .iter()
                .any(|s| s.format.color == Color32::from_rgb(120, 155, 125))
        );
    }

    #[test]
    fn source_location_is_persisted_with_the_tile() {
        let mut tile = SourceCodeTile::default();
        tile.open(SourceLocation {
            file: Utf8PathBuf::from("rtl/top.sv"),
            line: 17,
            column: 4,
        });
        let encoded = ron::to_string(&tile).unwrap();
        let restored: SourceCodeTile = ron::from_str(&encoded).unwrap();
        assert_eq!(restored.file, tile.file);
        assert_eq!((restored.line, restored.column), (17, 4));
    }

    #[test]
    fn systemverilog_lexer_carries_block_comments_across_lines() {
        let mut job = LayoutJob::default();
        let mut in_block_comment = false;
        append_systemverilog_line(
            &mut job,
            "/* declaration",
            Color32::WHITE,
            &mut in_block_comment,
        );
        append_systemverilog_line(
            &mut job,
            " */ logic ready; `define WIDTH 8",
            Color32::WHITE,
            &mut in_block_comment,
        );
        assert!(!in_block_comment);
        assert_eq!(job.text, "/* declaration */ logic ready; `define WIDTH 8");
        assert!(
            job.sections
                .iter()
                .any(|section| section.format.color == Color32::from_rgb(100, 200, 210))
        );
    }
}
