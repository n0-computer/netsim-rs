//! VM backend: subcommands and dispatch for patchbay-vm.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::Subcommand;
#[cfg(feature = "serve")]
use patchbay_server::DEFAULT_UI_BIND;
use patchbay_vm::VmOps;

use crate::test;
#[cfg(not(feature = "serve"))]
const DEFAULT_UI_BIND: &str = "127.0.0.1:7421";

/// Shared args for `patchbay run` (used by both top-level Run and Vm Run).
#[derive(Debug, Clone, clap::Args)]
pub struct VmRunArgs {
    /// One or more sim TOML files or directories containing `*.toml`.
    #[arg(required = true)]
    pub sims: Vec<PathBuf>,

    /// Work directory for logs, binaries, and results.
    #[arg(long, default_value = ".patchbay/work")]
    pub work_dir: PathBuf,

    /// Binary override in `<name>:<mode>:<value>` form.
    #[arg(long = "binary")]
    pub binary_overrides: Vec<String>,

    /// Stream live stdout/stderr lines with node prefixes.
    #[arg(short = 'v', long, default_value_t = false)]
    pub verbose: bool,

    /// Start embedded UI server and open browser.
    #[arg(long, default_value_t = false)]
    pub open: bool,

    /// Bind address for embedded UI server.
    #[arg(long, default_value = DEFAULT_UI_BIND)]
    pub bind: String,
}

/// VM sub-subcommands (mirrors patchbay-vm's standalone CLI).
#[derive(Subcommand)]
pub enum VmCommand {
    /// Boot or reuse VM and ensure mounts.
    Up {
        #[arg(long)]
        recreate: bool,
    },
    /// Stop VM and helper processes.
    Down,
    /// Show VM running status.
    Status,
    /// Best-effort cleanup of VM helper artifacts/processes.
    Cleanup,
    /// Execute command in the guest (SSH for QEMU, exec for container).
    Ssh {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    /// Run one or more sims in VM using guest patchbay binary.
    Run {
        #[command(flatten)]
        args: VmRunArgs,

        #[arg(long)]
        recreate: bool,
        #[arg(long, default_value = "latest")]
        patchbay_version: String,
    },
    /// Serve embedded UI + work directory over HTTP.
    Serve {
        #[arg(long, default_value = ".patchbay/work")]
        work_dir: PathBuf,
        /// Serve `<work-dir>/binaries/tests/testdir-current` instead of work_dir.
        #[arg(long, default_value_t = false)]
        testdir: bool,
        #[arg(long, default_value = DEFAULT_UI_BIND)]
        bind: String,
        #[arg(long, default_value_t = false)]
        open: bool,
    },
    /// Build and run tests in VM.
    Test {
        #[command(flatten)]
        args: test::TestArgs,
        #[arg(long, default_value_t = patchbay_vm::default_test_target())]
        target: String,
        #[arg(long)]
        recreate: bool,
    },
}

/// Dispatch VM subcommands to the patchbay-vm library.
pub async fn dispatch_vm(command: VmCommand, backend: patchbay_vm::Backend) -> Result<()> {
    let backend = backend.resolve();

    match command {
        VmCommand::Up { recreate } => backend.up(recreate),
        VmCommand::Down => backend.down(),
        VmCommand::Status => backend.status(),
        VmCommand::Cleanup => backend.cleanup(),
        VmCommand::Ssh { cmd } => backend.exec(cmd),
        VmCommand::Run {
            args,
            recreate,
            patchbay_version,
        } => {
            if args.open {
                #[cfg(feature = "serve")]
                {
                    let url = format!("http://{}", args.bind);
                    println!("patchbay UI: {url}");
                    let _ = open::that(&url);
                    let work = args.work_dir.clone();
                    let bind_clone = args.bind.clone();
                    tokio::spawn(async move {
                        if let Err(e) = patchbay_server::serve(work, &bind_clone).await {
                            tracing::error!("server error: {e}");
                        }
                    });
                }
                #[cfg(not(feature = "serve"))]
                bail!("--open requires the `serve` feature");
            }
            let vm_args = patchbay_vm::RunVmArgs {
                sim_inputs: args.sims,
                work_dir: args.work_dir.clone(),
                binary_overrides: args.binary_overrides,
                verbose: args.verbose,
                recreate,
                patchbay_version,
            };
            let res = backend.run_sims(vm_args);
            if args.open && res.is_ok() {
                println!("run finished; server still running (Ctrl-C to exit)");
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }
            res
        }
        VmCommand::Serve {
            work_dir,
            testdir,
            bind,
            open,
        } => {
            #[cfg(feature = "serve")]
            {
                let dir = if testdir {
                    work_dir
                        .join("binaries")
                        .join("tests")
                        .join("testdir-current")
                } else {
                    work_dir
                };
                println!("patchbay: serving {} at http://{bind}/", dir.display());
                if open {
                    let url = format!("http://{bind}");
                    let _ = open::that(&url);
                }
                patchbay_server::serve(dir, &bind).await
            }
            #[cfg(not(feature = "serve"))]
            {
                let _ = (&work_dir, &testdir, &bind, &open);
                bail!("serve requires the `serve` feature")
            }
        }
        VmCommand::Test {
            args,
            target,
            recreate,
        } => backend.run_tests(args.into_vm_args(target, recreate)),
    }
}

/// Run sims via VM backend (used by top-level `Run` on non-Linux).
#[allow(dead_code)] // Only called on non-Linux targets.
pub fn run_sims_vm(args: VmRunArgs, backend: patchbay_vm::Backend) -> Result<()> {
    let backend = backend.resolve();
    let vm_args = patchbay_vm::RunVmArgs {
        sim_inputs: args.sims,
        work_dir: args.work_dir,
        binary_overrides: args.binary_overrides,
        verbose: args.verbose,
        recreate: false,
        patchbay_version: "latest".to_string(),
    };
    backend.run_sims(vm_args)
}
