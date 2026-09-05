use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use eyre::Result;
use tracing::{
    Level,
    field::{Field, Visit},
};
use tracing_subscriber::Layer;

static RECORD_MUTEX: Mutex<Vec<LogMessage>> = Mutex::new(vec![]);
static SHOW_LOGS_ON_ERROR: AtomicBool = AtomicBool::new(false);
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

impl Visit for FieldVisitor<'_> {
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

        if *event.metadata().level() == Level::ERROR {
            SHOW_LOGS_ON_ERROR.store(true, Ordering::Release);
            if let Some(ctx) = crate::EGUI_CONTEXT.read().unwrap().as_ref() {
                ctx.request_repaint();
            }
        }
    }
}

pub(crate) fn take_error_notification() -> bool {
    SHOW_LOGS_ON_ERROR.swap(false, Ordering::AcqRel)
}

pub(crate) fn records() -> Vec<LogMessage> {
    RECORD_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Starts the logging and error handling.
///
/// Can be used by unittests to get more insights.
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

/// Starts the logging and error handling.
///
/// Can be used by unittests to get more insights.
#[cfg(target_arch = "wasm32")]
pub fn start_logging() -> Result<()> {
    use tracing_subscriber::{Registry, fmt, layer::SubscriberExt};
    use wasm_tracing::WasmLayer;

    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let subscriber = Registry::default()
        .with(fmt::layer().without_time().with_filter(filter.clone()))
        .with(WasmLayer::default())
        .with(EguiLogger {}.with_filter(filter));

    tracing::subscriber::set_global_default(subscriber).expect("unable to set global subscriber");

    Ok(())
}
