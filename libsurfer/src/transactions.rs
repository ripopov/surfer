use egui::{Layout, RichText};
use egui_extras::{Column, TableBody, TableBuilder};
use emath::Align;
use ftr_parser::types::Transaction;
use itertools::Itertools;
use num::BigUint;

use crate::message::Message;
use crate::transaction_container::StreamScopeRef;
use crate::transaction_container::TransactionStreamRef;
use crate::wave_data::ScopeType;
use crate::wave_data::WaveData;
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

pub fn draw_transaction_variable_list(
    msgs: &mut Vec<Message>,
    streams: &WaveData,
    ui: &mut egui::Ui,
    active_stream: &StreamScopeRef,
) {
    let inner = match streams.inner.as_transactions() {
        Some(tx) => tx,
        None => return,
    };
    match active_stream {
        StreamScopeRef::Root => {
            draw_transaction_root_variables(msgs, ui, inner);
        }
        StreamScopeRef::Stream(stream_ref) => {
            draw_transaction_stream_variables(msgs, ui, inner, stream_ref);
        }
        StreamScopeRef::Empty(_) => {}
    }
}

pub fn draw_transaction_root(msgs: &mut Vec<Message>, streams: &WaveData, ui: &mut egui::Ui) {
    egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        egui::Id::from("Streams"),
        false,
    )
    .show_header(ui, |ui| {
        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                let root_name = String::from("tr");
                let response = ui.add(egui::Button::selectable(
                    streams.active_scope == Some(ScopeType::StreamScope(StreamScopeRef::Root)),
                    root_name,
                ));

                response.clicked().then(|| {
                    msgs.push(Message::SetActiveScope(ScopeType::StreamScope(
                        StreamScopeRef::Root,
                    )));
                });
            },
        );
    })
    .body(|ui| {
        if let Some(tx_container) = streams.inner.as_transactions() {
            for (id, stream) in &tx_container.inner.tx_streams {
                let name = stream.name.clone();
                let response = ui.add(egui::Button::selectable(
                    streams.active_scope.as_ref().is_some_and(|s| {
                        if let ScopeType::StreamScope(StreamScopeRef::Stream(scope_stream)) = s {
                            scope_stream.stream_id == *id
                        } else {
                            false
                        }
                    }),
                    name.clone(),
                ));

                response.clicked().then(|| {
                    msgs.push(Message::SetActiveScope(ScopeType::StreamScope(
                        StreamScopeRef::Stream(TransactionStreamRef::new_stream(*id, name)),
                    )));
                });
            }
        }
    });
}

fn draw_transaction_stream_variables(
    msgs: &mut Vec<Message>,
    ui: &mut egui::Ui,
    inner: &crate::transaction_container::TransactionContainer,
    stream_ref: &TransactionStreamRef,
) {
    if let Some(stream) = inner.get_stream(stream_ref.stream_id) {
        let sorted_generators = stream.generators.iter().sorted_by(|a, b| {
            numeric_sort::cmp(
                &inner.get_generator(**a).unwrap().name,
                &inner.get_generator(**b).unwrap().name,
            )
        });
        for gen_id in sorted_generators {
            if let Some(generator) = inner.get_generator(*gen_id) {
                let gen_name = generator.name.clone();
                ui.with_layout(
                    Layout::top_down(Align::LEFT).with_cross_justify(true),
                    |ui| {
                        let response = ui.add(egui::Button::selectable(false, &gen_name));

                        response.clicked().then(|| {
                            msgs.push(Message::AddStreamOrGenerator(
                                TransactionStreamRef::new_gen(
                                    stream_ref.stream_id,
                                    *gen_id,
                                    gen_name,
                                ),
                            ));
                        });
                    },
                );
            }
        }
    } else {
        ui.label("Stream not found");
        tracing::warn!(
            "Stream ID {} not found in transaction container",
            stream_ref.stream_id
        );
    }
}

fn draw_transaction_root_variables(
    msgs: &mut Vec<Message>,
    ui: &mut egui::Ui,
    inner: &crate::transaction_container::TransactionContainer,
) {
    let streams = inner.get_streams();
    let sorted_streams = streams
        .iter()
        .sorted_by(|a, b| numeric_sort::cmp(&a.name, &b.name));
    for stream in sorted_streams {
        ui.with_layout(
            Layout::top_down(Align::LEFT).with_cross_justify(true),
            |ui| {
                let response = ui.add(egui::Button::selectable(false, stream.name.clone()));

                response.clicked().then(|| {
                    msgs.push(Message::AddStreamOrGenerator(
                        TransactionStreamRef::new_stream(stream.id, stream.name.clone()),
                    ));
                });
            },
        );
    }
}
