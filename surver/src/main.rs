//! Code for the `surver` executable.
use clap::Parser;
use eyre::Result;
use std::{
    fs::File,
    io::{BufRead, BufReader, stdout},
};
use tokio::runtime::Builder;
use tracing::subscriber::set_global_default;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, layer::SubscriberExt};

#[derive(Parser, Default)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (git: ", env!("VERGEN_GIT_DESCRIBE"), ")"), about)]
struct Args {
    #[clap(flatten)]
    file_group: FileGroup,
    /// Port on which server will listen
    #[clap(long)]
    port: Option<u16>,
    /// IP address to bind the server to
    #[clap(long)]
    bind_address: Option<String>,
    /// Token used by the client to authenticate to the server
    #[clap(long)]
    token: Option<String>,
}

#[derive(Debug, Default, clap::Args)]
#[group(required = true)]
pub struct FileGroup {
    /// Waveform files in VCD, FST, or GHW format.
    wave_files: Vec<String>,
    /// File with one wave form file name per line
    #[clap(long)]
    file: Option<String>,
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

    // Handle file lists
    let mut file_names = args.file_group.wave_files.clone();

    // Append file names from file
    if let Some(filename) = args.file_group.file {
        let file = File::open(filename).expect("no such file");
        let buf = BufReader::new(file);
        let mut files = buf
            .lines()
            .map(|l| l.expect("Could not parse line"))
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>();
        file_names.append(&mut files);
    }

    // Use CLI override if provided, otherwise use hardcoded defaults
    let bind_addr = args.bind_address.unwrap_or_else(|| "127.0.0.1".to_string());
    let port = args.port.unwrap_or(8911);

    runtime.block_on(surver::server_main(
        port,
        bind_addr,
        args.token,
        &file_names,
        None,
    ))
}
