use egui::{Layout, RichText};
use egui_extras::{Column, TableBuilder};
use emath::Align;
use ftr_parser::types::Transaction;
use num::BigUint;

use crate::SystemState;

impl SystemState {
    pub fn draw_focused_transaction_details(&self, ui: &mut egui::Ui) {
        if let Some(waves) = self.user.waves.as_ref() {
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
                            let focused_transaction =
                                waves.focused_transaction.1.as_ref().unwrap_or_else(|| {
                                    waves
                                        .inner
                                        .as_transactions()
                                        .unwrap()
                                        .get_transaction(
                                            waves.focused_transaction.0.as_ref().unwrap(),
                                        )
                                        .unwrap()
                                });
                            let row_height = 15.;
                            body.row(row_height, |mut row| {
                                row.col(|ui| {
                                    ui.label("Transaction ID");
                                });
                                row.col(|ui| {
                                    ui.label(focused_transaction.get_tx_id().to_string());
                                });
                            });
                            body.row(row_height, |mut row| {
                                row.col(|ui| {
                                    ui.label("Type");
                                });
                                row.col(|ui| {
                                    let gen = waves
                                        .inner
                                        .as_transactions()
                                        .unwrap()
                                        .get_generator(focused_transaction.get_gen_id())
                                        .unwrap();
                                    ui.label(gen.name.to_string());
                                });
                            });
                            body.row(row_height, |mut row| {
                                row.col(|ui| {
                                    ui.label("Start Time");
                                });
                                row.col(|ui| {
                                    ui.label(focused_transaction.get_start_time().to_string());
                                });
                            });
                            body.row(row_height, |mut row| {
                                row.col(|ui| {
                                    ui.label("End Time");
                                });
                                row.col(|ui| {
                                    ui.label(focused_transaction.get_end_time().to_string());
                                });
                            });
                            body.row(row_height + 5., |mut row| {
                                row.col(|ui| {
                                    ui.heading("Attributes");
                                });
                            });

                            body.row(row_height + 3., |mut row| {
                                row.col(|ui| {
                                    ui.label(RichText::new("Name").size(15.));
                                });
                                row.col(|ui| {
                                    ui.label(RichText::new("Value").size(15.));
                                });
                            });

                            for attr in &focused_transaction.attributes {
                                body.row(row_height, |mut row| {
                                    row.col(|ui| {
                                        ui.label(attr.name.to_string());
                                    });
                                    row.col(|ui| {
                                        ui.label(attr.value().to_string());
                                    });
                                });
                            }

                            if !focused_transaction.inc_relations.is_empty() {
                                body.row(row_height + 5., |mut row| {
                                    row.col(|ui| {
                                        ui.heading("Incoming Relations");
                                    });
                                });

                                body.row(row_height + 3., |mut row| {
                                    row.col(|ui| {
                                        ui.label(RichText::new("Source Tx").size(15.));
                                    });
                                    row.col(|ui| {
                                        ui.label(RichText::new("Sink Tx").size(15.));
                                    });
                                });

                                for rel in &focused_transaction.inc_relations {
                                    body.row(row_height, |mut row| {
                                        row.col(|ui| {
                                            ui.label(rel.source_tx_id.to_string());
                                        });
                                        row.col(|ui| {
                                            ui.label(rel.sink_tx_id.to_string());
                                        });
                                    });
                                }
                            }

                            if !focused_transaction.out_relations.is_empty() {
                                body.row(row_height + 5., |mut row| {
                                    row.col(|ui| {
                                        ui.heading("Outgoing Relations");
                                    });
                                });

                                body.row(row_height + 3., |mut row| {
                                    row.col(|ui| {
                                        ui.label(RichText::new("Source Tx").size(15.));
                                    });
                                    row.col(|ui| {
                                        ui.label(RichText::new("Sink Tx").size(15.));
                                    });
                                });

                                for rel in &focused_transaction.out_relations {
                                    body.row(row_height, |mut row| {
                                        row.col(|ui| {
                                            ui.label(rel.source_tx_id.to_string());
                                        });
                                        row.col(|ui| {
                                            ui.label(rel.sink_tx_id.to_string());
                                        });
                                    });
                                }
                            }
                        });
                },
            );
        }
    }
}

pub fn calculate_rows_of_stream(
    transactions: &Vec<Transaction>,
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
