mod common;
mod container;
mod qemu;
mod util;

fn default_test_target() -> String {
    if std::env::consts::ARCH == "aarch64" {
        "aarch64-unknown-linux-musl".to_string()
    } else {
        "x86_64-unknown-linux-musl".to_string()
    }
}

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use patchbay_server::DEFAULT_UI_BIND;

use common::{RunVmArgs, TestVmArgs};

/// VM backend selection.
#[derive(Clone, Debug, ValueEnum)]
enum Backend {
    /// Auto-detect: prefer `container` on macOS Apple Silicon, fall back to QEMU.
    Auto,
    /// QEMU with a full Debian cloud image and SSH access.
    Qemu,
    /// Apple `container` CLI (macOS 26 + Apple Silicon only).
    Container,
}

#[derive(Parser)]
#[command(name = "patchbay-vm", about = "Standalone VM runner for patchbay")]
struct Cli {
    /// Which VM backend to use.
    #[arg(long, default_value = "auto", global = true)]
    backend: Backend,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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
        #[arg(required = true)]
        sims: Vec<PathBuf>,
        #[arg(long, default_value = ".patchbay-work")]
        work_dir: PathBuf,
        #[arg(long = "binary")]
        binary_overrides: Vec<String>,
        #[arg(short = 'v', long, default_value_t = false)]
        verbose: bool,
        #[arg(long)]
        recreate: bool,
        #[arg(long, default_value = "latest")]
        patchbay_version: String,
        #[arg(long, default_value_t = false)]
        open: bool,
        #[arg(long, default_value = DEFAULT_UI_BIND)]
        bind: String,
    },
    /// Serve embedded UI + work directory over HTTP.
    Serve {
        #[arg(long, default_value = ".patchbay-work")]
        work_dir: PathBuf,
        /// Serve `<work-dir>/binaries/tests/testdir-current` instead of work_dir.
        ///
        /// In the VM, test binaries live under `<work-dir>/binaries/tests/` and
        /// the testdir crate writes output next to the executable.
        #[arg(long, default_value_t = false)]
        testdir: bool,
        #[arg(long, default_value = DEFAULT_UI_BIND)]
        bind: String,
        #[arg(long, default_value_t = false)]
        open: bool,
    },
    /// Build and run tests in VM (replaces legacy test-vm flow).
    ///
    /// Positional FILTER is passed to each test binary as a name filter
    /// (like `cargo test <filter>`). Extra args after `--` go to cargo
    /// during the build and to each test binary at runtime.
    Test {
        /// Test name filter (passed to test binaries at runtime).
        #[arg()]
        filter: Option<String>,
        #[arg(long, default_value_t = default_test_target())]
        target: String,
        #[arg(short = 'p', long = "package")]
        packages: Vec<String>,
        #[arg(long = "test")]
        tests: Vec<String>,
        #[arg(short = 'j', long)]
        jobs: Option<u32>,
        #[arg(short = 'F', long)]
        features: Vec<String>,
        #[arg(long)]
        release: bool,
        #[arg(long)]
        lib: bool,
        #[arg(long)]
        no_fail_fast: bool,
        #[arg(long)]
        recreate: bool,
        #[arg(last = true)]
        cargo_args: Vec<String>,
    },
}

/// Resolve `Backend::Auto` into a concrete backend.
fn resolve_backend(b: Backend) -> Backend {
    match b {
        Backend::Auto => {
            if std::env::consts::OS == "macos"
                && std::env::consts::ARCH == "aarch64"
                && common::command_exists("container").unwrap_or(false)
            {
                Backend::Container
            } else {
                Backend::Qemu
            }
        }
        other => other,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    patchbay_utils::init_tracing();
    let cli = Cli::parse();
    let backend = resolve_backend(cli.backend);

    match cli.command {
        Command::Up { recreate } => match backend {
            Backend::Container => container::up_cmd(recreate),
            _ => qemu::up_cmd(recreate),
        },
        Command::Down => match backend {
            Backend::Container => container::down_cmd(),
            _ => qemu::down_cmd(),
        },
        Command::Status => match backend {
            Backend::Container => container::status_cmd(),
            _ => qemu::status_cmd(),
        },
        Command::Cleanup => match backend {
            Backend::Container => container::cleanup_cmd(),
            _ => qemu::cleanup_cmd(),
        },
        Command::Ssh { cmd } => match backend {
            Backend::Container => container::exec_cmd_cli(cmd),
            _ => qemu::ssh_cmd_cli(cmd),
        },
        Command::Run {
            sims,
            work_dir,
            binary_overrides,
            verbose,
            recreate,
            patchbay_version,
            open,
            bind,
        } => {
            if open {
                let url = format!("http://{bind}");
                println!("patchbay UI: {url}");
                let _ = open::that(&url);
                let work = work_dir.clone();
                tokio::spawn(async move {
                    if let Err(e) = patchbay_server::serve(work, &bind).await {
                        tracing::error!("server error: {e}");
                    }
                });
            }
            let args = RunVmArgs {
                sim_inputs: sims,
                work_dir,
                binary_overrides,
                verbose,
                recreate,
                patchbay_version,
            };
            let res = match backend {
                Backend::Container => container::run_sims(args),
                _ => qemu::run_sims_in_vm(args),
            };
            if open && res.is_ok() {
                println!("run finished; server still running (Ctrl-C to exit)");
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            }
            res
        }
        Command::Serve {
            work_dir,
            testdir,
            bind,
            open,
        } => {
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
        Command::Test {
            filter,
            target,
            packages,
            tests,
            jobs,
            features,
            release,
            lib,
            no_fail_fast,
            recreate,
            mut cargo_args,
        } => {
            if let Some(j) = jobs {
                cargo_args.extend(["--jobs".into(), j.to_string()]);
            }
            for f in features {
                cargo_args.extend(["--features".into(), f]);
            }
            if release {
                cargo_args.push("--release".into());
            }
            if lib {
                cargo_args.push("--lib".into());
            }
            if no_fail_fast {
                cargo_args.push("--no-fail-fast".into());
            }
            let args = TestVmArgs {
                filter,
                target,
                packages,
                tests,
                recreate,
                cargo_args,
            };
            match backend {
                Backend::Container => container::run_tests(args),
                _ => qemu::run_tests_in_vm(args),
            }
        }
    }
}
