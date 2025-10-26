use egui::{Layout, RichText};
use egui_extras::{Column, TableBody, TableBuilder};
use emath::Align;
use ftr_parser::types::Transaction;
use num::BigUint;

use crate::SystemState;

impl SystemState {
    pub fn draw_focused_transaction_details(&self, ui: &mut egui::Ui) {
        let Some(waves) = self.user.waves.as_ref() else {
            return;
        };
        let Some(transactions) = waves.inner.as_transactions() else {
            return;
        };
        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                ui.label("Focused Transaction Details");
                let column_width = ui.available_width() / 2.;
                TableBuilder::new(ui)
                    .column(Column::exact(column_width))
                    .column(Column::auto())
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.heading("Properties");
                        });
                    })
                    .body(|mut body| {
                        // --- Helpers ----------------------------------------------------
                        let row_height = 15.;
                        let section_gap = 5.;
                        let subheader_gap = 3.;
                        let subheader_size = 15.;

                        let focused_transaction =
                            waves.focused_transaction.1.as_ref().unwrap_or_else(|| {
                                transactions
                                    .get_transaction(waves.focused_transaction.0.as_ref().unwrap())
                                    .unwrap()
                            });
                        table_row(
                            &mut body,
                            row_height,
                            "Transaction ID",
                            focused_transaction.get_tx_id().to_string(),
                        );
                        table_row(&mut body, row_height, "Type", {
                            let gen = transactions
                                .get_generator(focused_transaction.get_gen_id())
                                .unwrap();
                            gen.name.to_string()
                        });
                        table_row(
                            &mut body,
                            row_height,
                            "Start Time",
                            focused_transaction.get_start_time().to_string(),
                        );
                        table_row(
                            &mut body,
                            row_height,
                            "End Time",
                            focused_transaction.get_end_time().to_string(),
                        );
                        section_header(&mut body, row_height + section_gap, "Attributes");
                        subheader(
                            &mut body,
                            row_height + subheader_gap,
                            "Name",
                            "Value",
                            subheader_size,
                        );

                        for attr in &focused_transaction.attributes {
                            table_row(&mut body, row_height, &attr.name, attr.value().to_string());
                        }

                        if !focused_transaction.inc_relations.is_empty() {
                            section_header(
                                &mut body,
                                row_height + section_gap,
                                "Incoming Relations",
                            );
                            subheader(
                                &mut body,
                                row_height + subheader_gap,
                                "Source Tx",
                                "Sink Tx",
                                subheader_size,
                            );

                            for rel in &focused_transaction.inc_relations {
                                table_row(
                                    &mut body,
                                    row_height,
                                    &rel.source_tx_id.to_string(),
                                    rel.sink_tx_id.to_string(),
                                );
                            }
                        }

                        if !focused_transaction.out_relations.is_empty() {
                            section_header(
                                &mut body,
                                row_height + section_gap,
                                "Outgoing Relations",
                            );
                            subheader(
                                &mut body,
                                row_height + subheader_gap,
                                "Source Tx",
                                "Sink Tx",
                                subheader_size,
                            );

                            for rel in &focused_transaction.out_relations {
                                table_row(
                                    &mut body,
                                    row_height,
                                    &rel.source_tx_id.to_string(),
                                    rel.sink_tx_id.to_string(),
                                );
                            }
                        }
                    });
            },
        );
    }
}

pub fn calculate_rows_of_stream(
    transactions: &[Transaction],
    last_times_on_row: &mut Vec<(BigUint, BigUint)>,
) {
    for transaction in transactions {
        let mut curr_row = 0;
        let start_time = transaction.get_start_time();
        let end_time = transaction.get_end_time();

        while start_time > last_times_on_row[curr_row].0
            && start_time < last_times_on_row[curr_row].1
        {
            curr_row += 1;
            if last_times_on_row.len() <= curr_row {
                last_times_on_row.push((BigUint::ZERO, BigUint::ZERO));
            }
        }
        last_times_on_row[curr_row] = (start_time, end_time);
    }
}

fn table_row(body: &mut TableBody, h: f32, key: &str, val: String) {
    body.row(h, |mut row| {
        row.col(|ui| {
            ui.label(key);
        });
        row.col(|ui| {
            ui.label(val);
        });
    });
}

fn section_header(body: &mut TableBody, h: f32, title: &str) {
    body.row(h, |mut row| {
        row.col(|ui| {
            ui.heading(title);
        });
    });
}

fn subheader(body: &mut TableBody, h: f32, left: &str, right: &str, size: f32) {
    body.row(h, |mut row| {
        row.col(|ui| {
            ui.label(RichText::new(left).size(size));
        });
        row.col(|ui| {
            ui.label(RichText::new(right).size(size));
        });
    });
}
