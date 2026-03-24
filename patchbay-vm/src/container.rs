//! Apple `container` CLI backend for patchbay-vm.
//!
//! Uses Apple's Containerization framework (macOS 26 + Apple Silicon) to run a
//! lightweight Linux VM via `container run`. Guest commands execute through
//! `container exec` instead of SSH.

use std::{
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};

use crate::common::{
    self, abspath, assemble_guest_build_overrides, build_and_collect_test_binaries,
    cargo_target_dir, default_musl_target, ensure_guest_runner_binary, env_or, log_msg,
    need_cmd, run_checked, stage_test_binaries, to_guest_sim_path, RunVmArgs, TestVmArgs,
    GUEST_PREPARE_SCRIPT,
};
use crate::util::stage_binary_overrides;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CONTAINER_STATE_DIR: &str = ".container-vm";
const DEFAULT_CONTAINER_NAME: &str = "patchbay";
const DEFAULT_IMAGE: &str = "debian:trixie-slim";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ContainerConfig {
    name: String,
    image: String,
    mem_mb: String,
    cpus: String,
    workspace: PathBuf,
    target_dir: PathBuf,
    work_dir: PathBuf,
    state_root: PathBuf,
    recreate: bool,
}

impl ContainerConfig {
    fn from_args(args: &RunVmArgs) -> Result<Self> {
        let cwd = std::env::current_dir().context("get cwd")?;
        let target_dir = match cargo_target_dir() {
            Ok(dir) => dir,
            Err(_) => {
                let current_exe = std::env::current_exe().context("resolve current executable")?;
                let profile_dir = current_exe
                    .parent()
                    .context("current executable has no parent")?;
                let base = profile_dir
                    .parent()
                    .context("current executable profile dir has no parent")?;
                base.to_path_buf()
            }
        };

        Ok(Self {
            name: env_or("CONTAINER_VM_NAME", DEFAULT_CONTAINER_NAME),
            image: env_or("CONTAINER_VM_IMAGE", DEFAULT_IMAGE),
            mem_mb: env_or("CONTAINER_VM_MEM_MB", common::DEFAULT_MEM_MB),
            cpus: env_or("CONTAINER_VM_CPUS", &common::default_cpus()),
            workspace: cwd,
            target_dir,
            work_dir: abspath(&args.work_dir)?,
            state_root: std::env::current_dir()?.join(CONTAINER_STATE_DIR),
            recreate: args.recreate,
        })
    }

    fn from_defaults() -> Result<Self> {
        let cwd = std::env::current_dir().context("get cwd")?;
        let target_dir = match cargo_target_dir() {
            Ok(dir) => dir,
            Err(_) => cwd.join("target"),
        };
        let default_work = cwd.join(".patchbay-work");

        Ok(Self {
            name: env_or("CONTAINER_VM_NAME", DEFAULT_CONTAINER_NAME),
            image: env_or("CONTAINER_VM_IMAGE", DEFAULT_IMAGE),
            mem_mb: env_or("CONTAINER_VM_MEM_MB", common::DEFAULT_MEM_MB),
            cpus: env_or("CONTAINER_VM_CPUS", &common::default_cpus()),
            workspace: cwd,
            target_dir,
            work_dir: PathBuf::from(env_or(
                "CONTAINER_VM_WORK_DIR",
                &default_work.display().to_string(),
            )),
            state_root: std::env::current_dir()?.join(CONTAINER_STATE_DIR),
            recreate: false,
        })
    }

    fn state_dir(&self) -> PathBuf {
        self.state_root.join(&self.name)
    }

    fn runtime_file(&self) -> PathBuf {
        self.state_dir().join("runtime.env")
    }
}

fn log(msg: &str) {
    log_msg("container", msg);
}

// ---------------------------------------------------------------------------
// Public entrypoints
// ---------------------------------------------------------------------------

pub fn up_cmd(recreate: bool) -> Result<()> {
    let mut cfg = ContainerConfig::from_defaults()?;
    cfg.recreate = recreate;
    up(&mut cfg)
}

pub fn down_cmd() -> Result<()> {
    let cfg = ContainerConfig::from_defaults()?;
    down(&cfg)
}

pub fn status_cmd() -> Result<()> {
    let cfg = ContainerConfig::from_defaults()?;
    println!("backend: container");
    println!("container-name: {}", cfg.name);
    println!(
        "running: {}",
        if is_running(&cfg)? { "yes" } else { "no" }
    );
    if cfg.runtime_file().exists() {
        println!("runtime: {}", cfg.runtime_file().display());
        let text = std::fs::read_to_string(cfg.runtime_file())?;
        print!("{text}");
    }
    Ok(())
}

pub fn cleanup_cmd() -> Result<()> {
    let cfg = ContainerConfig::from_defaults()?;
    if is_running(&cfg)? {
        down(&cfg)?;
    }
    common::remove_if_exists(&cfg.state_dir())?;
    Ok(())
}

/// Maps the `Ssh` subcommand to `container exec`.
pub fn exec_cmd_cli(cmd: Vec<String>) -> Result<()> {
    let cfg = ContainerConfig::from_defaults()?;
    if cmd.is_empty() {
        bail!("exec: missing command");
    }
    let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();
    exec_cmd(&cfg, &refs)
}

pub fn run_sims(args: RunVmArgs) -> Result<()> {
    let mut cfg = ContainerConfig::from_args(&args)?;
    up(&mut cfg)?;
    prepare_guest(&cfg)?;
    run_in_guest(&cfg, &args)?;
    Ok(())
}

pub fn run_tests(args: TestVmArgs) -> Result<()> {
    let mut cfg = ContainerConfig::from_defaults()?;
    cfg.recreate = args.recreate;
    let target_dir = cargo_target_dir()?;

    let (test_bins, vm_result) = std::thread::scope(|s| {
        let build = s.spawn(|| {
            build_and_collect_test_binaries(
                &target_dir,
                &args.target,
                &args.packages,
                &args.tests,
                &args.cargo_args,
            )
        });
        let setup = s.spawn(|| {
            up(&mut cfg)?;
            prepare_guest(&cfg)
        });
        (build.join(), setup.join())
    });
    let test_bins = test_bins
        .map_err(|_| anyhow!("build thread panicked"))?
        .context("building test binaries")?;
    vm_result
        .map_err(|_| anyhow!("container setup thread panicked"))?
        .context("container setup")?;

    if test_bins.is_empty() {
        bail!("no test binaries were built for target {}", args.target);
    }

    let staged = stage_test_binaries(&cfg.work_dir, &test_bins)?;

    let forward_envs: &[&str] = &[
        "RUST_LOG",
        "RUST_BACKTRACE",
        "PATCHBAY_OUTDIR",
        "PATCHBAY_SIM",
    ];
    let mut env_pairs: Vec<String> = forward_envs
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|val| format!("{name}={val}")))
        .collect();
    env_pairs.push("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());

    let mut passed = 0usize;
    let mut failed = 0usize;
    for guest_bin in staged {
        let mut run_args: Vec<String> = Vec::new();
        if !env_pairs.is_empty() {
            run_args.push("env".into());
            run_args.extend(env_pairs.iter().cloned());
        }
        run_args.push(guest_bin.clone());
        if let Some(ref f) = args.filter {
            run_args.push(f.clone());
        }
        let run_refs: Vec<&str> = run_args.iter().map(|s| s.as_str()).collect();
        match exec_cmd(&cfg, &run_refs) {
            Ok(()) => {
                passed += 1;
                println!("[test] PASS {guest_bin}");
            }
            Err(err) => {
                failed += 1;
                println!("[test] FAIL {guest_bin}: {err}");
            }
        }
    }
    println!("[test] summary: passed={passed} failed={failed}");
    if failed > 0 {
        bail!("{} test binaries failed in container", failed);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

fn up(cfg: &mut ContainerConfig) -> Result<()> {
    need_cmd("container")?;
    std::fs::create_dir_all(cfg.state_dir())
        .with_context(|| format!("create {}", cfg.state_dir().display()))?;
    std::fs::create_dir_all(&cfg.target_dir)?;
    std::fs::create_dir_all(&cfg.work_dir)?;

    log(&format!("workspace={}", cfg.workspace.display()));
    log(&format!("target={}", cfg.target_dir.display()));
    log(&format!("work={}", cfg.work_dir.display()));

    if cfg.recreate && is_running(cfg)? {
        log("recreate requested; stopping existing container");
        down(cfg)?;
    }

    if is_running(cfg)? {
        check_running_mount_paths(cfg)?;
        log("container already running");
        return Ok(());
    }

    log(&format!("starting container from {}", cfg.image));
    start_container(cfg)?;
    wait_for_ready(cfg)?;
    persist_runtime(cfg)?;
    log(&format!("{} ready", cfg.name));
    Ok(())
}

fn down(cfg: &ContainerConfig) -> Result<()> {
    if !is_running(cfg)? {
        log(&format!("{} is not running", cfg.name));
        return Ok(());
    }
    log(&format!("stopping {}", cfg.name));
    let _ = Command::new("container")
        .args(["stop", &cfg.name])
        .status();
    // Remove the stopped container so the name can be reused.
    let _ = Command::new("container")
        .args(["rm", &cfg.name])
        .status();
    common::remove_if_exists(&cfg.runtime_file())?;
    log(&format!("{} stopped", cfg.name));
    Ok(())
}

fn is_running(cfg: &ContainerConfig) -> Result<bool> {
    let output = Command::new("container")
        .args(["inspect", &cfg.name])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) => {
            if !out.status.success() {
                return Ok(false);
            }
            let text = String::from_utf8_lossy(&out.stdout);
            Ok(text.contains("running") || text.contains("Running"))
        }
        Err(_) => Ok(false),
    }
}

fn start_container(cfg: &ContainerConfig) -> Result<()> {
    // Remove any stopped container with the same name.
    let _ = Command::new("container")
        .args(["rm", &cfg.name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let mut cmd = Command::new("container");
    cmd.args(["run", "-d", "--name", &cfg.name]);

    cmd.args(["--cpus", &cfg.cpus]);
    cmd.args(["--memory", &format!("{}M", cfg.mem_mb)]);

    // Mount workspace (read-only) at /app.
    cmd.args([
        "--mount",
        &format!(
            "type=bind,source={},target=/app,readonly",
            cfg.workspace.display()
        ),
    ]);
    // Mount target dir (read-only) at /target.
    cmd.args([
        "--mount",
        &format!(
            "type=bind,source={},target=/target,readonly",
            cfg.target_dir.display()
        ),
    ]);
    // Mount work dir (read-write) at /work.
    cmd.args([
        "--mount",
        &format!(
            "type=bind,source={},target=/work",
            cfg.work_dir.display()
        ),
    ]);

    cmd.arg(&cfg.image);
    // Keep the container alive with a long sleep so we can exec into it.
    cmd.args(["sleep", "infinity"]);

    run_checked(&mut cmd, "container run")
}

fn wait_for_ready(cfg: &ContainerConfig) -> Result<()> {
    log("waiting for container to be ready...");
    for _ in 0..60 {
        if is_running(cfg)? {
            // Verify we can actually exec into it.
            let ok = Command::new("container")
                .args(["exec", &cfg.name, "true"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                log("container is ready");
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    bail!(
        "container '{}' did not become ready within 30 seconds",
        cfg.name
    )
}

// ---------------------------------------------------------------------------
// Guest interaction
// ---------------------------------------------------------------------------

fn exec_cmd(cfg: &ContainerConfig, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("container");
    cmd.args(["exec", &cfg.name]);
    cmd.args(args);
    run_checked(&mut cmd, "container exec")
}

fn prepare_guest(cfg: &ContainerConfig) -> Result<()> {
    exec_cmd(cfg, &["bash", "-lc", GUEST_PREPARE_SCRIPT])
}

fn run_in_guest(cfg: &ContainerConfig, args: &RunVmArgs) -> Result<()> {
    let guest_exe =
        ensure_guest_runner_binary(&cfg.work_dir, &cfg.target_dir, &args.patchbay_version)?;
    let auto_build_overrides = assemble_guest_build_overrides(&cfg.target_dir, args)?;
    let staged_overrides = stage_binary_overrides(
        &args.binary_overrides,
        &cfg.work_dir,
        &cfg.target_dir,
        default_musl_target(),
    )?;

    // No `sudo` needed; container exec runs as root by default.
    let mut parts = vec![
        "env".to_string(),
        "NETSIM_IN_VM=1".to_string(),
        "NETSIM_TARGET_DIR=/target".to_string(),
    ];
    if let Ok(rust_log) = std::env::var("NETSIM_RUST_LOG") {
        parts.push(format!("NETSIM_RUST_LOG={rust_log}"));
    }
    if let Ok(rust_log) = std::env::var("RUST_LOG") {
        parts.push(format!("RUST_LOG={rust_log}"));
    }
    parts.extend([
        guest_exe,
        "run".to_string(),
        "--work-dir".to_string(),
        "/work".to_string(),
    ]);

    for ov in &auto_build_overrides {
        parts.push("--binary".to_string());
        parts.push(ov.clone());
    }
    for ov in &staged_overrides {
        parts.push("--binary".to_string());
        parts.push(ov.clone());
    }
    if args.verbose {
        parts.push("-v".to_string());
    }
    for sim in &args.sim_inputs {
        parts.push(to_guest_sim_path(&cfg.workspace, sim)?);
    }

    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    exec_cmd(cfg, &refs)
}

// ---------------------------------------------------------------------------
// State persistence
// ---------------------------------------------------------------------------

fn persist_runtime(cfg: &ContainerConfig) -> Result<()> {
    let text = format!(
        "backend=container\nworkspace={}\ntarget_dir={}\nwork_dir={}\n",
        cfg.workspace.display(),
        cfg.target_dir.display(),
        cfg.work_dir.display(),
    );
    std::fs::write(cfg.runtime_file(), text)
        .with_context(|| format!("write {}", cfg.runtime_file().display()))
}

fn check_running_mount_paths(cfg: &ContainerConfig) -> Result<()> {
    if !cfg.runtime_file().exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(cfg.runtime_file())
        .with_context(|| format!("read {}", cfg.runtime_file().display()))?;
    let mut running_workspace = None;
    let mut running_target = None;
    let mut running_work = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("workspace=") {
            running_workspace = Some(v.to_string());
        }
        if let Some(v) = line.strip_prefix("target_dir=") {
            running_target = Some(v.to_string());
        }
        if let Some(v) = line.strip_prefix("work_dir=") {
            running_work = Some(v.to_string());
        }
    }

    if running_workspace.as_deref() != Some(cfg.workspace.to_string_lossy().as_ref()) {
        bail!(
            "container already running with workspace '{}', requested '{}' (use --recreate)",
            running_workspace.unwrap_or_default(),
            cfg.workspace.display()
        );
    }
    if running_target.as_deref() != Some(cfg.target_dir.to_string_lossy().as_ref()) {
        bail!(
            "container already running with target dir '{}', requested '{}' (use --recreate)",
            running_target.unwrap_or_default(),
            cfg.target_dir.display()
        );
    }
    if running_work.as_deref() != Some(cfg.work_dir.to_string_lossy().as_ref()) {
        bail!(
            "container already running with work dir '{}', requested '{}' (use --recreate)",
            running_work.unwrap_or_default(),
            cfg.work_dir.display()
        );
    }
    Ok(())
}
