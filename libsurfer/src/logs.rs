use std::{collections::BTreeMap, sync::Mutex};

use ecolor::Color32;
use egui::{RichText, TextWrapMode};
use egui_extras::{Column, TableBuilder, TableRow};
use eyre::Result;
use tracing::{
    Level,
    field::{Field, Visit},
};
use tracing_subscriber::Layer;

use crate::{SystemState, message::Message};

static RECORD_MUTEX: Mutex<Vec<LogMessage>> = Mutex::new(vec![]);

#[macro_export]
macro_rules! try_log_error {
    ($expr:expr, $what:expr $(,)?) => {
        if let Err(e) = $expr {
            error!("{}: {}", $what, e)
        }
    };
}

#[derive(Clone)]
pub struct LogMessage {
    pub name: String,
    pub msg: String,
    pub level: Level,
}

struct EguiLogger {}

struct FieldVisitor<'a>(&'a mut BTreeMap<String, String>);

impl<'a> Visit for FieldVisitor<'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for EguiLogger
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldVisitor(&mut fields));

        RECORD_MUTEX
            .lock()
            .expect("Failed to lock logger. Thread poisoned?")
            .push(LogMessage {
                name: event.metadata().module_path().unwrap_or("-").to_string(),
                msg: fields.get("message").cloned().unwrap_or("-".to_string()),
                level: *event.metadata().level(),
            });
    }
}

impl SystemState {
    pub fn draw_log_window(&self, ctx: &egui::Context, msgs: &mut Vec<Message>) {
        let mut open = true;
        egui::Window::new("Logs")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);

                egui::ScrollArea::new([true, false]).show(ui, |ui| {
                    TableBuilder::new(ui)
                        .column(Column::auto().resizable(true))
                        .column(Column::auto().resizable(true))
                        .column(Column::remainder())
                        .vscroll(true)
                        .stick_to_bottom(true)
                        .header(20.0, |mut header| {
                            header.col(|ui| {
                                ui.heading("Level");
                            });
                            header.col(|ui| {
                                ui.heading("Source");
                            });
                            header.col(|ui| {
                                ui.heading("Message");
                            });
                        })
                        .body(|body| {
                            let records = RECORD_MUTEX.lock().unwrap();
                            let heights = records
                                .iter()
                                .map(|record| {
                                    let height = record.msg.lines().count() as f32;

                                    height * 15.
                                })
                                .collect::<Vec<_>>();

                            body.heterogeneous_rows(heights.into_iter(), |mut row: TableRow| {
                                let record = &records[row.index()];
                                row.col(|ui| {
                                    let (color, text) = match record.level {
                                        Level::ERROR => (Color32::RED, "Error"),
                                        Level::WARN => (Color32::YELLOW, "Warn"),
                                        Level::INFO => (Color32::GREEN, "Info"),
                                        Level::DEBUG => (Color32::BLUE, "Debug"),
                                        Level::TRACE => (Color32::GRAY, "Trace"),
                                    };

                                    ui.colored_label(color, text);
                                });
                                row.col(|ui| {
                                    ui.label(
                                        RichText::new(record.name.clone())
                                            .color(Color32::GRAY)
                                            .monospace(),
                                    );
                                });
                                row.col(|ui| {
                                    ui.label(RichText::new(record.msg.clone()).monospace());
                                });
                            });
                        });
                })
            });
        if !open {
            msgs.push(Message::SetLogsVisible(false));
        }
    }
}

/// Starts the logging and error handling. Can be used by unittests to get more insights.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_logging() -> Result<()> {
    use std::io::stdout;

    use tracing_subscriber::{Registry, fmt, layer::SubscriberExt};

    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let subscriber = Registry::default()
        .with(
            fmt::layer()
                .without_time()
                .with_writer(stdout)
                .with_filter(filter.clone()),
        )
        .with(EguiLogger {}.with_filter(filter));

    tracing::subscriber::set_global_default(subscriber).expect("unable to set global subscriber");

    Ok(())
}
