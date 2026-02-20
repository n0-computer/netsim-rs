//! Runs the `netsim` CLI entrypoint.

mod sim;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use netsim::check_caps;

#[derive(Parser)]
#[command(name = "netsim", about = "Run a netsim simulation")]
struct Cli {
    /// One or more sim TOML files or directories containing `*.toml`.
    #[arg(required = true)]
    sims: Vec<PathBuf>,

    /// Work directory for logs, binaries, and results.
    #[arg(long, default_value = ".netsim-work")]
    work_dir: PathBuf,

    /// Binary override in `<name>:<mode>:<value>` form.
    ///
    /// Modes:
    /// - `build` (build from local checkout path)
    /// - `fetch` (download from URL)
    /// - `path`  (copy local path into workdir/bins and use it)
    #[arg(long = "binary")]
    binary_overrides: Vec<String>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    netsim::Lab::init_tracing();
    check_caps()?;

    let cli = Cli::parse();
    sim::run_sims(cli.sims, cli.work_dir, cli.binary_overrides).await
}
