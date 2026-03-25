//! Unified CLI entrypoint for patchbay simulations (native and VM).

mod compare;
mod init;
mod test;
#[cfg(feature = "upload")]
mod upload;
mod util;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use patchbay::check_caps;
use patchbay_runner::sim;
#[cfg(feature = "serve")]
use patchbay_server::DEFAULT_UI_BIND;
#[cfg(not(feature = "serve"))]
const DEFAULT_UI_BIND: &str = "127.0.0.1:7421";
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(name = "patchbay", about = "Run a patchbay simulation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one or more sims locally.
    Run {
        /// One or more sim TOML files or directories containing `*.toml`.
        #[arg()]
        sims: Vec<PathBuf>,

        /// Work directory for logs, binaries, and results.
        #[arg(long, default_value = ".patchbay/work")]
        work_dir: PathBuf,

        /// Binary override in `<name>:<mode>:<value>` form.
        #[arg(long = "binary")]
        binary_overrides: Vec<String>,

        /// Do not build binaries; resolve expected artifacts from target dirs.
        #[arg(long, default_value_t = false)]
        no_build: bool,
        /// Stream live stdout/stderr lines with node prefixes.
        #[arg(short = 'v', long, default_value_t = false)]
        verbose: bool,

        /// Start embedded UI server and open browser.
        #[arg(long, default_value_t = false)]
        open: bool,

        /// Bind address for embedded UI server.
        #[arg(long, default_value = DEFAULT_UI_BIND)]
        bind: String,

        /// Project root directory for resolving binaries and cargo builds.
        /// Defaults to the current working directory.
        #[arg(long)]
        project_root: Option<PathBuf>,

        /// Per-sim timeout (e.g. "120s", "5m"). Sims that exceed this are aborted.
        #[arg(long)]
        timeout: Option<String>,
    },
    /// Resolve sims and build all required assets without running simulations.
    Prepare {
        /// One or more sim TOML files or directories containing `*.toml`.
        #[arg()]
        sims: Vec<PathBuf>,
        /// Work directory for caches and prepared outputs.
        #[arg(long, default_value = ".patchbay/work")]
        work_dir: PathBuf,
        /// Binary override in `<name>:<mode>:<value>` form.
        #[arg(long = "binary")]
        binary_overrides: Vec<String>,
        /// Do not build binaries; resolve expected artifacts from target dirs.
        #[arg(long, default_value_t = false)]
        no_build: bool,

        /// Project root directory for resolving binaries and cargo builds.
        /// Defaults to the current working directory.
        #[arg(long)]
        project_root: Option<PathBuf>,
    },
    /// Serve embedded devtools UI over HTTP for a lab output directory.
    #[cfg(feature = "serve")]
    Serve {
        /// Output directory containing lab run subdirectories.
        ///
        /// Ignored when `--testdir` is set.
        #[arg(default_value = ".patchbay/work")]
        outdir: PathBuf,
        /// Serve `<cargo-target-dir>/testdir-current` instead of a path.
        ///
        /// Uses `cargo metadata` to locate the target directory.
        #[arg(long, default_value_t = false)]
        testdir: bool,
        /// Bind address for HTTP server.
        #[arg(long, default_value = DEFAULT_UI_BIND)]
        bind: String,
        /// Open browser after server start.
        #[arg(long, default_value_t = false)]
        open: bool,
    },
    /// Build topology from sim/topology config for interactive namespace debugging.
    Inspect {
        /// Sim TOML or topology TOML file path.
        input: PathBuf,
        /// Work directory for inspect session metadata.
        #[arg(long, default_value = ".patchbay/work")]
        work_dir: PathBuf,
    },
    /// Run a command inside a node namespace from an inspect session.
    RunIn {
        /// Device or router name from the inspected topology.
        node: String,
        /// Inspect session id (defaults to `$NETSIM_INSPECT`).
        #[arg(long)]
        inspect: Option<String>,
        /// Work directory containing inspect session metadata.
        #[arg(long, default_value = ".patchbay/work")]
        work_dir: PathBuf,
        /// Command and args to execute in the node namespace.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        cmd: Vec<String>,
    },
    /// Run tests (delegates to cargo test on native, VM test flow on VM).
    Test {
        #[command(flatten)]
        args: test::TestArgs,

        /// Force VM backend.
        #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
        vm: Option<String>,
    },
    /// Compare test or sim results across git refs.
    Compare {
        #[command(subcommand)]
        command: CompareCommand,
    },
    /// Upload a run/compare directory to a patchbay-server instance.
    Upload {
        /// Directory to upload (e.g. .patchbay/work/compare-20260325_120000).
        dir: PathBuf,
        /// Project name for scoping on the server.
        #[arg(long, env = "PATCHBAY_PROJECT")]
        project: String,
        /// Server URL (e.g. https://patchbay.example.com).
        #[arg(long, env = "PATCHBAY_URL")]
        url: String,
        /// API key for authentication.
        #[arg(long, env = "PATCHBAY_API_KEY")]
        api_key: String,
    },
    /// VM management and simulation execution.
    #[cfg(feature = "vm")]
    Vm {
        #[command(subcommand)]
        command: VmCommand,
        /// Which VM backend to use.
        #[arg(long, default_value = "auto", global = true)]
        backend: patchbay_vm::Backend,
    },
}

#[derive(Subcommand)]
enum CompareCommand {
    /// Compare test results between git refs.
    Test {
        /// First git ref (compare against worktree if only one given).
        #[arg(long = "ref", required = true)]
        left_ref: String,

        /// Second git ref (if omitted, compare left_ref against current worktree).
        #[arg(long = "ref2")]
        right_ref: Option<String>,

        #[command(flatten)]
        args: test::TestArgs,
    },
    /// Compare sim results between git refs.
    Run {
        /// Sim TOML files or directories.
        #[arg(required = true)]
        sims: Vec<PathBuf>,

        /// First git ref.
        #[arg(long = "ref", required = true)]
        left_ref: String,

        /// Second git ref.
        #[arg(long = "ref2")]
        right_ref: Option<String>,
    },
}

/// VM sub-subcommands (mirrors patchbay-vm's standalone CLI).
#[cfg(feature = "vm")]
#[derive(Subcommand)]
enum VmCommand {
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
        #[arg(long, default_value = ".patchbay/work")]
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
        /// Test name filter (passed to test binaries at runtime).
        #[arg()]
        filter: Option<String>,
        #[arg(long, default_value_t = patchbay_vm::default_test_target())]
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

fn resolve_project_root(opt: Option<PathBuf>) -> Result<PathBuf> {
    match opt {
        Some(p) => Ok(p),
        None => std::env::current_dir().context("resolve current directory"),
    }
}

fn main() -> Result<()> {
    patchbay::init_userns()?;
    tokio_main()
}

#[tokio::main(flavor = "current_thread")]
async fn tokio_main() -> Result<()> {
    patchbay_utils::init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            sims,
            work_dir,
            binary_overrides,
            no_build,
            verbose,
            open,
            bind: _bind,
            project_root,
            timeout,
        } => {
            let sim_timeout = timeout
                .map(|s| sim::steps::parse_duration(&s))
                .transpose()
                .context("invalid --timeout value")?;
            if open {
                #[cfg(feature = "serve")]
                {
                    let bind_addr = _bind.clone();
                    let work = work_dir.clone();
                    tokio::spawn(async move {
                        if let Err(e) = patchbay_server::serve(work, &bind_addr).await {
                            tracing::error!("server error: {e}");
                        }
                    });
                    println!("patchbay: http://{_bind}/");
                    let url = format!("http://{_bind}/");
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                }
                #[cfg(not(feature = "serve"))]
                bail!("--open requires the `serve` feature");
            }
            let project_root = resolve_project_root(project_root)?;
            let sims = resolve_sim_args(sims, &project_root)?;
            let res = sim::run_sims(
                sims,
                work_dir,
                binary_overrides,
                verbose,
                Some(project_root),
                no_build,
                sim_timeout,
            )
            .await;
            if open && res.is_ok() {
                println!("run finished; server still running (Ctrl-C to exit)");
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }
            res
        }
        Command::Prepare {
            sims,
            work_dir,
            binary_overrides,
            no_build,
            project_root,
        } => {
            let project_root = resolve_project_root(project_root)?;
            let sims = resolve_sim_args(sims, &project_root)?;
            sim::prepare_sims(
                sims,
                work_dir,
                binary_overrides,
                Some(project_root),
                no_build,
            )
            .await
        }
        #[cfg(feature = "serve")]
        Command::Serve {
            outdir,
            testdir,
            bind,
            open,
        } => {
            let dir = if testdir {
                resolve_testdir_native()?
            } else {
                outdir
            };
            println!("patchbay: serving {} at http://{bind}/", dir.display());
            if open {
                let url = format!("http://{bind}/");
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            }
            patchbay_server::serve(dir, &bind).await
        }
        Command::Inspect { input, work_dir } => inspect_command(input, work_dir).await,
        Command::RunIn {
            node,
            inspect,
            work_dir,
            cmd,
        } => run_in_command(node, inspect, work_dir, cmd),
        Command::Test { args, vm } => {
            #[cfg(feature = "vm")]
            if let Some(vm_backend) = vm {
                let backend = match vm_backend.as_str() {
                    "auto" => patchbay_vm::Backend::Auto.resolve(),
                    "qemu" => patchbay_vm::Backend::Qemu,
                    "container" => patchbay_vm::Backend::Container,
                    other => bail!("unknown VM backend: {other}"),
                };
                return test::run_vm(args, backend);
            }
            #[cfg(not(feature = "vm"))]
            if vm.is_some() {
                bail!("VM support not compiled (enable the `vm` feature)");
            }
            test::run_native(args)
        }
        Command::Compare { command } => {
            let cwd = std::env::current_dir().context("get cwd")?;
            match command {
                CompareCommand::Test { left_ref, right_ref, args } => {
                    let right_label = right_ref.as_deref().unwrap_or("worktree");
                    println!("patchbay compare test: {} \u{2194} {}", left_ref, right_label);

                    // Set up worktrees
                    let left_dir = compare::setup_worktree(&left_ref, &cwd)?;
                    let right_dir = if let Some(ref r) = right_ref {
                        compare::setup_worktree(r, &cwd)?
                    } else {
                        cwd.clone()
                    };

                    // Run tests sequentially
                    println!("Running tests in {} ...", left_ref);
                    let (left_results, _left_output) = compare::run_tests_in_dir(
                        &left_dir, &args,
                    )?;

                    println!("Running tests in {} ...", right_label);
                    let (right_results, _right_output) = compare::run_tests_in_dir(
                        &right_dir, &args,
                    )?;

                    // Compare
                    let summary = compare::compare_results(&left_ref, right_label, &left_results, &right_results);
                    compare::print_summary(&left_ref, right_label, &left_results, &right_results, &summary);

                    // Write manifest
                    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
                    let compare_dir = cwd.join(".patchbay/work").join(format!("compare-{ts}"));
                    std::fs::create_dir_all(&compare_dir)?;
                    let manifest = compare::CompareManifest {
                        left_ref: left_ref.clone(),
                        right_ref: right_label.to_string(),
                        timestamp: ts,
                        project: std::env::var("PATCHBAY_PROJECT").ok(),
                        left_results,
                        right_results,
                        summary,
                    };
                    let manifest_path = compare_dir.join("summary.json");
                    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
                    println!("\nManifest: {}", manifest_path.display());

                    // Cleanup worktrees
                    compare::cleanup_worktree(&left_dir)?;
                    if right_ref.is_some() {
                        compare::cleanup_worktree(&right_dir)?;
                    }

                    if manifest.summary.regressions > 0 {
                        bail!("{} regressions detected", manifest.summary.regressions);
                    }
                    Ok(())
                }
                CompareCommand::Run { sims: _, left_ref: _, right_ref: _ } => {
                    // TODO: implement compare run (sim comparison)
                    bail!("compare run is not yet implemented");
                }
            }
        }
        Command::Upload { dir, project, url, api_key } => {
            if !dir.exists() {
                bail!("directory does not exist: {}", dir.display());
            }
            #[cfg(feature = "upload")]
            { upload::upload(&dir, &project, &url, &api_key) }
            #[cfg(not(feature = "upload"))]
            { let _ = (&dir, &project, &url, &api_key); bail!("upload support not compiled in (enable the `upload` feature)") }
        }
        #[cfg(feature = "vm")]
        Command::Vm { command, backend } => dispatch_vm(command, backend).await,
    }
}

/// Dispatch VM subcommands to the patchbay-vm library.
#[cfg(feature = "vm")]
async fn dispatch_vm(command: VmCommand, backend: patchbay_vm::Backend) -> Result<()> {
    let backend = backend.resolve();

    match command {
        VmCommand::Up { recreate } => backend.up(recreate),
        VmCommand::Down => backend.down(),
        VmCommand::Status => backend.status(),
        VmCommand::Cleanup => backend.cleanup(),
        VmCommand::Ssh { cmd } => backend.exec(cmd),
        VmCommand::Run {
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
                let bind_clone = bind.clone();
                tokio::spawn(async move {
                    if let Err(e) = patchbay_server::serve(work, &bind_clone).await {
                        tracing::error!("server error: {e}");
                    }
                });
            }
            let args = patchbay_vm::RunVmArgs {
                sim_inputs: sims,
                work_dir,
                binary_overrides,
                verbose,
                recreate,
                patchbay_version,
            };
            let res = backend.run_sims(args);
            if open && res.is_ok() {
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
        VmCommand::Test {
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
            cargo_args,
        } => {
            let test_args = test::TestArgs {
                filter,
                ignored: false,
                ignored_only: false,
                packages,
                tests,
                jobs,
                features,
                release,
                lib,
                no_fail_fast,
                extra_args: cargo_args,
            };
            backend.run_tests(test_args.into_vm_args(target, recreate))
        }
    }
}

/// When no sim paths are given on the CLI, look for `patchbay.toml` or
/// `.patchbay.toml` in the project root and use its `simulations` path.
fn resolve_sim_args(sims: Vec<PathBuf>, project_root: &Path) -> Result<Vec<PathBuf>> {
    if !sims.is_empty() {
        return Ok(sims);
    }
    let candidates = [
        project_root.join("patchbay.toml"),
        project_root.join(".patchbay.toml"),
    ];
    for path in &candidates {
        if path.is_file() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("read {}", path.display()))?;
            let cfg: PatchbayConfig =
                toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
            let sims_dir = project_root.join(&cfg.simulations);
            if !sims_dir.exists() {
                bail!(
                    "{}: simulations path '{}' does not exist",
                    path.display(),
                    sims_dir.display()
                );
            }
            println!("patchbay: using simulations from {}", sims_dir.display());
            return Ok(vec![sims_dir]);
        }
    }
    bail!(
        "no sim files specified and no patchbay.toml found in {}",
        project_root.display()
    )
}

#[derive(Deserialize)]
struct PatchbayConfig {
    /// Path to sims directory (relative to project root).
    simulations: String,
}

/// Resolve `testdir-current` inside the cargo target directory.
///
/// Runs `cargo metadata` to find the target directory, then appends
/// `testdir-current`. This matches the convention used by the `testdir`
/// crate when running tests natively.
#[cfg(feature = "serve")]
fn resolve_testdir_native() -> Result<PathBuf> {
    let output = ProcessCommand::new("cargo")
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()
        .context("failed to run `cargo metadata`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cargo metadata failed: {stderr}");
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse cargo metadata")?;
    let target_dir = meta["target_directory"]
        .as_str()
        .ok_or_else(|| anyhow!("cargo metadata missing target_directory"))?;
    Ok(PathBuf::from(target_dir).join("testdir-current"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InspectSession {
    prefix: String,
    root_ns: String,
    node_namespaces: HashMap<String, String>,
    node_ips_v4: HashMap<String, String>,
    node_keeper_pids: HashMap<String, u32>,
}

fn inspect_dir(work_dir: &std::path::Path) -> PathBuf {
    work_dir.join("inspect")
}

fn inspect_session_path(work_dir: &std::path::Path, prefix: &str) -> PathBuf {
    inspect_dir(work_dir).join(format!("{prefix}.json"))
}

fn env_key_suffix(name: &str) -> String {
    patchbay::util::sanitize_for_env_key(name)
}

fn load_topology_for_inspect(
    input: &std::path::Path,
) -> Result<(patchbay::config::LabConfig, bool)> {
    let text =
        std::fs::read_to_string(input).with_context(|| format!("read {}", input.display()))?;
    let value: toml::Value =
        toml::from_str(&text).with_context(|| format!("parse TOML {}", input.display()))?;
    let is_sim =
        value.get("sim").is_some() || value.get("step").is_some() || value.get("binary").is_some();
    if is_sim {
        let sim: sim::SimFile =
            toml::from_str(&text).with_context(|| format!("parse sim {}", input.display()))?;
        let topo = sim::topology::load_topology(&sim, input)
            .with_context(|| format!("load topology from sim {}", input.display()))?;
        Ok((topo, true))
    } else {
        let topo: patchbay::config::LabConfig =
            toml::from_str(&text).with_context(|| format!("parse topology {}", input.display()))?;
        Ok((topo, false))
    }
}

fn keeper_commmand() -> ProcessCommand {
    let mut cmd = ProcessCommand::new("sh");
    cmd.args(["-lc", "while :; do sleep 3600; done"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd
}

async fn inspect_command(input: PathBuf, work_dir: PathBuf) -> Result<()> {
    check_caps()?;

    let (topo, is_sim) = load_topology_for_inspect(&input)?;
    let lab = patchbay_runner::Lab::from_config(topo.clone())
        .await
        .with_context(|| format!("build lab config from {}", input.display()))?;

    let mut node_namespaces = HashMap::new();
    let mut node_ips_v4 = HashMap::new();
    let mut node_keeper_pids = HashMap::new();

    for router in &topo.router {
        let name = router.name.clone();
        let r = lab
            .router_by_name(&name)
            .with_context(|| format!("unknown router '{name}'"))?;
        let child = r.spawn_command_sync(keeper_commmand())?;
        node_keeper_pids.insert(name.clone(), child.id());
        node_namespaces.insert(name.clone(), r.ns().to_string());
        if let Some(ip) = r.uplink_ip() {
            node_ips_v4.insert(name, ip.to_string());
        }
    }
    for name in topo.device.keys() {
        let d = lab
            .device_by_name(name)
            .with_context(|| format!("unknown device '{name}'"))?;
        let child = d.spawn_command_sync(keeper_commmand())?;
        node_keeper_pids.insert(name.clone(), child.id());
        node_namespaces.insert(name.clone(), d.ns().to_string());
        if let Some(ip) = d.ip() {
            node_ips_v4.insert(name.clone(), ip.to_string());
        }
    }

    let prefix = lab.prefix().to_string();
    let session = InspectSession {
        prefix: prefix.clone(),
        root_ns: lab.ix().ns(),
        node_namespaces,
        node_ips_v4,
        node_keeper_pids,
    };

    let session_dir = inspect_dir(&work_dir);
    std::fs::create_dir_all(&session_dir)
        .with_context(|| format!("create {}", session_dir.display()))?;
    let session_path = inspect_session_path(&work_dir, &prefix);
    std::fs::write(&session_path, serde_json::to_vec_pretty(&session)?)
        .with_context(|| format!("write {}", session_path.display()))?;

    let mut keys = session
        .node_namespaces
        .keys()
        .map(|k| k.to_string())
        .collect::<Vec<_>>();
    keys.sort();

    println!(
        "inspect ready: {} ({})",
        session.prefix,
        if is_sim { "sim" } else { "topology" }
    );
    println!("session file: {}", session_path.display());
    println!("export NETSIM_INSPECT={}", session.prefix);
    println!("export NETSIM_INSPECT_FILE={}", session_path.display());
    for key in &keys {
        if let Some(ns) = session.node_namespaces.get(key) {
            println!("export NETSIM_NS_{}={ns}", env_key_suffix(key));
        }
        if let Some(ip) = session.node_ips_v4.get(key) {
            println!("export NETSIM_IP_{}={ip}", env_key_suffix(key));
        }
    }
    println!("inspect active; press Ctrl-C to stop and clean up");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn resolve_inspect_ref(inspect: Option<String>) -> Result<String> {
    if let Some(value) = inspect {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("--inspect must not be empty");
        }
        return Ok(trimmed.to_string());
    }
    let from_env = std::env::var("NETSIM_INSPECT")
        .context("missing inspect session; set --inspect or NETSIM_INSPECT")?;
    let trimmed = from_env.trim();
    if trimmed.is_empty() {
        bail!("NETSIM_INSPECT is set but empty");
    }
    Ok(trimmed.to_string())
}

fn load_inspect_session(work_dir: &std::path::Path, inspect_ref: &str) -> Result<InspectSession> {
    let as_path = PathBuf::from(inspect_ref);
    let session_path = if as_path.extension().and_then(|v| v.to_str()) == Some("json")
        || inspect_ref.contains('/')
    {
        as_path
    } else {
        inspect_session_path(work_dir, inspect_ref)
    };
    let bytes = std::fs::read(&session_path)
        .with_context(|| format!("read inspect session {}", session_path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse inspect session {}", session_path.display()))
}

fn run_in_command(
    node: String,
    inspect: Option<String>,
    work_dir: PathBuf,
    cmd: Vec<String>,
) -> Result<()> {
    check_caps()?;
    if cmd.is_empty() {
        bail!("run-in: missing command");
    }
    let inspect_ref = resolve_inspect_ref(inspect)?;
    let session = load_inspect_session(&work_dir, &inspect_ref)?;
    let pid = *session.node_keeper_pids.get(&node).ok_or_else(|| {
        anyhow!(
            "node '{}' is not in inspect session '{}'",
            node,
            session.prefix
        )
    })?;

    let mut proc = ProcessCommand::new("nsenter");
    proc.arg("-U")
        .arg("-t")
        .arg(pid.to_string())
        .arg("-n")
        .arg("--")
        .arg(&cmd[0]);
    if cmd.len() > 1 {
        proc.args(&cmd[1..]);
    }
    let status = proc
        .status()
        .context("run command with nsenter for inspect session")?;
    if !status.success() {
        bail!("run-in command exited with status {}", status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn env_key_suffix_normalizes_names() {
        assert_eq!(env_key_suffix("relay"), "relay");
        assert_eq!(env_key_suffix("fetcher-1"), "fetcher_1");
    }

    #[test]
    fn inspect_session_path_uses_prefix_json() {
        let base = PathBuf::from("/tmp/patchbay-work");
        let path = inspect_session_path(&base, "lab-p123");
        assert!(path.ends_with("inspect/lab-p123.json"));
    }

    fn write_temp_file(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, body).expect("write file");
        path
    }

    #[test]
    fn inspect_loader_detects_sim_input() {
        let root = std::env::temp_dir().join(format!(
            "patchbay-inspect-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let sim_path = write_temp_file(
            &root,
            "sims/case.toml",
            "[sim]\nname='x'\n\n[[router]]\nname='relay'\n\n[device.fetcher.eth0]\ngateway='relay'\n",
        );
        let (_topo, is_sim) = load_topology_for_inspect(&sim_path).expect("load sim topology");
        assert!(is_sim);
    }

    #[test]
    fn inspect_loader_detects_topology_input() {
        let root = std::env::temp_dir().join(format!(
            "patchbay-inspect-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let topo_path = write_temp_file(
            &root,
            "topos/lab.toml",
            "[[router]]\nname='relay'\n\n[device.fetcher.eth0]\ngateway='relay'\n",
        );
        let (_topo, is_sim) = load_topology_for_inspect(&topo_path).expect("load direct topology");
        assert!(!is_sim);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn iperf_sim_writes_results_with_mbps() {
        let root = std::env::temp_dir().join(format!(
            "patchbay-iperf-run-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp workdir");
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let sim_path = workspace_root.join("iroh-integration/patchbay/sims/iperf-1to1-public.toml");
        let project_root = workspace_root;
        sim::run_sims(
            vec![sim_path],
            root.clone(),
            vec![],
            false,
            Some(project_root),
            false,
            None,
        )
        .await
        .expect("run iperf sim");

        let run_root = std::fs::canonicalize(root.join("latest")).expect("resolve latest");
        let results_path = run_root
            .join("iperf-1to1-public-baseline")
            .join("results.json");
        let text = std::fs::read_to_string(&results_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", results_path.display()));
        let json: serde_json::Value = serde_json::from_str(&text).expect("parse results");
        let step = &json["steps"][0];
        let down_bytes: f64 = step["down_bytes"]
            .as_str()
            .expect("down_bytes should be present")
            .parse()
            .expect("down_bytes should be numeric");
        let duration: f64 = step["duration"]
            .as_str()
            .expect("duration should be present")
            .parse::<u64>()
            .map(|us| us as f64 / 1_000_000.0)
            .unwrap_or_else(|_| {
                step["duration"]
                    .as_str()
                    .unwrap()
                    .parse::<f64>()
                    .expect("duration as float")
            });
        let mb_s = down_bytes / (duration * 1_000_000.0);
        assert!(mb_s > 0.0, "expected mb_s > 0, got {mb_s}");
    }
}
