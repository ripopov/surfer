//! Code for the `surver` executable.
use clap::Parser;
use eyre::Result;
use std::io::stdout;
use tokio::runtime::Builder;
use tracing::subscriber::set_global_default;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, layer::SubscriberExt};

#[derive(clap::Parser, Default)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (git: ", env!("VERGEN_GIT_DESCRIBE"), ")"), about)]
struct Args {
    /// Waveform file in VCD, FST, or GHW format.
    wave_file: String,
    /// Port on which server will listen
    #[clap(long)]
    port: Option<u16>,
    /// IP address to bind the server to
    #[clap(long)]
    bind_address: Option<String>,
    /// Token used by the client to authenticate to the server
    #[clap(long)]
    token: Option<String>,
    #[clap(long)]
    /// Seconds to guard against repeated reloads, default 1 s
    reload_guard: Option<u64>,
}

/// Starts the logging and error handling. Can be used by unittests to get more insights.
pub fn start_logging() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let subscriber = Registry::default().with(
        fmt::layer()
            .without_time()
            .with_writer(stdout)
            .with_filter(filter.clone()),
    );

    set_global_default(subscriber).expect("unable to set global subscriber");

    Ok(())
}

fn main() -> Result<()> {
    start_logging()?;

    let runtime = Builder::new_current_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();

    // parse arguments
    let args = Args::parse();

    // Use CLI override if provided, otherwise use hardcoded defaults
    let bind_addr = args.bind_address.unwrap_or_else(|| "127.0.0.1".to_string());
    let port = args.port.unwrap_or(8911);
    let reload_guard = args.reload_guard.unwrap_or(1);

    runtime.block_on(surver::server_main(
        port,
        bind_addr,
        args.token,
        args.wave_file,
        None,
        reload_guard,
    ))
}
