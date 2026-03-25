use std::{
    fs::File,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};

use crate::{
    common::{
        self, abspath, assemble_guest_build_overrides, build_and_collect_test_binaries,
        cargo_target_dir, default_musl_target, ensure_guest_runner_binary, env_or, fnv1a64,
        force_kill_pid, is_arm64_host, kill_pid, log_msg, need_cmd, pid_alive, read_pid,
        remove_if_exists, run_checked, sanitize_filename, shell_join, stage_test_binaries,
        to_guest_sim_path, RunVmArgs, TestVmArgs, GUEST_PREPARE_SCRIPT,
    },
    util::stage_binary_overrides,
};

// ---------------------------------------------------------------------------
// QEMU-specific constants
// ---------------------------------------------------------------------------

const VM_STATE_DIR: &str = ".qemu-vm";
const DEFAULT_VM_NAME: &str = "patchbay-vm";
const DEFAULT_IMAGE_URL_X86: &str =
    "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-genericcloud-amd64.qcow2";
const DEFAULT_IMAGE_URL_ARM64: &str =
    "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-genericcloud-arm64.qcow2";
const DEFAULT_DISK_GB: &str = "40";
const DEFAULT_SSH_USER: &str = "dev";
const DEFAULT_QEMU_BIN_X86: &str = "qemu-system-x86_64";
const DEFAULT_QEMU_BIN_ARM64: &str = "qemu-system-aarch64";
const DEFAULT_SSH_PORT: &str = "2222";
const DEFAULT_SEED_PORT: &str = "8555";

const DEFAULT_VIRTIOFSD: [&str; 5] = [
    "/usr/lib/virtiofsd",
    "/usr/libexec/virtiofsd",
    "/usr/lib/qemu/virtiofsd",
    "/usr/bin/virtiofsd",
    "/opt/homebrew/libexec/virtiofsd",
];

const DISK_IMG: &str = "disk.qcow2";
const SEED_IMG: &str = "seed.iso";
const SEED_DIR: &str = "seed-http";
const USER_DATA: &str = "user-data";
const META_DATA: &str = "meta-data";
const NETWORK_CFG: &str = "network-config";
const SEED_MODE: &str = "seed-mode";
const SEED_PID: &str = "seed-http.pid";
const WORKSPACE_SOCK: &str = "workspace.vfs.sock";
const TARGET_SOCK: &str = "target.vfs.sock";
const WORK_SOCK: &str = "work.vfs.sock";
const WORKSPACE_VFS_PID: &str = "workspace.virtiofsd.pid";
const TARGET_VFS_PID: &str = "target.virtiofsd.pid";
const WORK_VFS_PID: &str = "work.virtiofsd.pid";
const QEMU_PID: &str = "qemu.pid";
const SERIAL_LOG: &str = "serial.log";
const SSH_KEY: &str = "id_ed25519";
const KNOWN_HOSTS: &str = "known_hosts";
const RUNTIME_ENV: &str = "runtime.env";

// ---------------------------------------------------------------------------
// QEMU-specific helpers
// ---------------------------------------------------------------------------

fn default_qemu_bin() -> &'static str {
    if is_arm64_host() {
        DEFAULT_QEMU_BIN_ARM64
    } else {
        DEFAULT_QEMU_BIN_X86
    }
}

fn default_image_url() -> &'static str {
    if is_arm64_host() {
        DEFAULT_IMAGE_URL_ARM64
    } else {
        DEFAULT_IMAGE_URL_X86
    }
}

fn log(msg: &str) {
    log_msg("qemu", msg);
}

// ---------------------------------------------------------------------------
// VmConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VmConfig {
    vm_name: String,
    image_url: String,
    mem_mb: String,
    cpus: String,
    disk_gb: String,
    ssh_user: String,
    qemu_bin: String,
    ssh_port: String,
    seed_port: String,
    workspace: PathBuf,
    target_dir: PathBuf,
    work_dir: PathBuf,
    state_root: PathBuf,
    shared_image_dir: PathBuf,
    recreate: bool,
    virtiofsd_bin: Option<PathBuf>,
    fs_mode: String,
}

impl VmConfig {
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
            vm_name: env_or("QEMU_VM_NAME", DEFAULT_VM_NAME),
            image_url: env_or("QEMU_VM_IMAGE_URL", default_image_url()),
            mem_mb: env_or("QEMU_VM_MEM_MB", common::DEFAULT_MEM_MB),
            cpus: env_or("QEMU_VM_CPUS", &common::default_cpus()),
            disk_gb: env_or("QEMU_VM_DISK_GB", DEFAULT_DISK_GB),
            ssh_user: env_or("QEMU_VM_SSH_USER", DEFAULT_SSH_USER),
            qemu_bin: env_or("QEMU_VM_QEMU_BIN", default_qemu_bin()),
            ssh_port: env_or("QEMU_VM_SSH_PORT", DEFAULT_SSH_PORT),
            seed_port: env_or("QEMU_VM_SEED_PORT", DEFAULT_SEED_PORT),
            workspace: cwd,
            target_dir,
            work_dir: abspath(&args.work_dir)?,
            state_root: std::env::current_dir()?.join(VM_STATE_DIR),
            shared_image_dir: shared_image_dir()?,
            recreate: args.recreate,
            virtiofsd_bin: std::env::var("QEMU_VM_VIRTIOFSD_BIN")
                .ok()
                .map(PathBuf::from),
            fs_mode: "9p".to_string(),
        })
    }

    fn from_cleanup_defaults() -> Result<Self> {
        let cwd = std::env::current_dir().context("get cwd")?;
        let target_dir = match cargo_target_dir() {
            Ok(dir) => dir,
            Err(_) => cwd.join("target"),
        };
        let default_work = cwd.join(".patchbay-work");

        Ok(Self {
            vm_name: env_or("QEMU_VM_NAME", DEFAULT_VM_NAME),
            image_url: env_or("QEMU_VM_IMAGE_URL", default_image_url()),
            mem_mb: env_or("QEMU_VM_MEM_MB", common::DEFAULT_MEM_MB),
            cpus: env_or("QEMU_VM_CPUS", &common::default_cpus()),
            disk_gb: env_or("QEMU_VM_DISK_GB", DEFAULT_DISK_GB),
            ssh_user: env_or("QEMU_VM_SSH_USER", DEFAULT_SSH_USER),
            qemu_bin: env_or("QEMU_VM_QEMU_BIN", default_qemu_bin()),
            ssh_port: env_or("QEMU_VM_SSH_PORT", DEFAULT_SSH_PORT),
            seed_port: env_or("QEMU_VM_SEED_PORT", DEFAULT_SEED_PORT),
            workspace: cwd.clone(),
            target_dir,
            work_dir: PathBuf::from(env_or(
                "QEMU_VM_WORK_DIR",
                &default_work.display().to_string(),
            )),
            state_root: cwd.join(VM_STATE_DIR),
            shared_image_dir: shared_image_dir()?,
            recreate: false,
            virtiofsd_bin: std::env::var("QEMU_VM_VIRTIOFSD_BIN")
                .ok()
                .map(PathBuf::from),
            fs_mode: "9p".to_string(),
        })
    }

    fn state_root(&self) -> PathBuf {
        self.state_root.clone()
    }

    fn vm_dir(&self) -> PathBuf {
        self.state_root().join(&self.vm_name)
    }

    fn p(&self, name: &str) -> PathBuf {
        self.vm_dir().join(name)
    }

    fn base_img(&self) -> PathBuf {
        self.shared_image_dir.join(base_image_name(&self.image_url))
    }

    fn disk_img(&self) -> PathBuf {
        self.p(DISK_IMG)
    }

    fn seed_img(&self) -> PathBuf {
        self.p(SEED_IMG)
    }

    fn seed_dir(&self) -> PathBuf {
        self.p(SEED_DIR)
    }

    fn user_data(&self) -> PathBuf {
        self.p(USER_DATA)
    }

    fn meta_data(&self) -> PathBuf {
        self.p(META_DATA)
    }

    fn network_cfg(&self) -> PathBuf {
        self.p(NETWORK_CFG)
    }

    fn seed_mode_file(&self) -> PathBuf {
        self.p(SEED_MODE)
    }

    fn seed_pid_file(&self) -> PathBuf {
        self.p(SEED_PID)
    }

    fn workspace_sock(&self) -> PathBuf {
        self.p(WORKSPACE_SOCK)
    }

    fn target_sock(&self) -> PathBuf {
        self.p(TARGET_SOCK)
    }

    fn work_sock(&self) -> PathBuf {
        self.p(WORK_SOCK)
    }

    fn workspace_vfs_pid(&self) -> PathBuf {
        self.p(WORKSPACE_VFS_PID)
    }

    fn target_vfs_pid(&self) -> PathBuf {
        self.p(TARGET_VFS_PID)
    }

    fn work_vfs_pid(&self) -> PathBuf {
        self.p(WORK_VFS_PID)
    }

    fn pid_file(&self) -> PathBuf {
        self.p(QEMU_PID)
    }

    fn serial_log(&self) -> PathBuf {
        self.p(SERIAL_LOG)
    }

    fn ssh_key(&self) -> PathBuf {
        self.p(SSH_KEY)
    }

    fn known_hosts(&self) -> PathBuf {
        self.p(KNOWN_HOSTS)
    }

    fn runtime_file(&self) -> PathBuf {
        self.p(RUNTIME_ENV)
    }
}

// ---------------------------------------------------------------------------
// Public entrypoints
// ---------------------------------------------------------------------------

pub fn run_sims_in_vm(args: RunVmArgs) -> Result<()> {
    let mut vm = VmConfig::from_args(&args)?;
    up(&mut vm)?;
    prepare_vm_guest(&vm)?;
    run_in_guest(&vm, &args)?;
    Ok(())
}

/// Stops the local VM if it is running and cleans leftover VM helper processes.
pub fn stop_vm_if_running() -> Result<()> {
    let vm = VmConfig::from_cleanup_defaults()?;
    down(&vm)
}

/// Builds tests for the target and runs discovered test binaries inside the VM.
pub fn run_tests_in_vm(args: TestVmArgs) -> Result<()> {
    let mut vm = VmConfig::from_cleanup_defaults()?;
    vm.recreate = args.recreate;
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
        let vm_setup = s.spawn(|| {
            up(&mut vm)?;
            prepare_vm_guest(&vm)
        });
        (build.join(), vm_setup.join())
    });
    let test_bins = test_bins
        .map_err(|_| anyhow!("build thread panicked"))?
        .context("building test binaries")?;
    vm_result
        .map_err(|_| anyhow!("vm setup thread panicked"))?
        .context("vm setup")?;

    if test_bins.is_empty() {
        bail!("no test binaries were built for target {}", args.target);
    }

    let staged = stage_test_binaries(&vm.work_dir, &test_bins)?;

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
        let rc = ssh_cmd_status(&vm, &run_refs);
        match rc {
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
        bail!("{} test binaries failed in VM", failed);
    }
    Ok(())
}

pub fn up_cmd(recreate: bool) -> Result<()> {
    let mut vm = VmConfig::from_cleanup_defaults()?;
    vm.recreate = recreate;
    up(&mut vm)
}

pub fn down_cmd() -> Result<()> {
    stop_vm_if_running()
}

pub fn cleanup_cmd() -> Result<()> {
    let vm = VmConfig::from_cleanup_defaults()?;
    cleanup_seed_server(&vm)?;
    cleanup_virtiofsd(&vm)?;
    remove_if_exists(&vm.pid_file())?;
    remove_if_exists(&vm.runtime_file())?;
    Ok(())
}

pub fn status_cmd() -> Result<()> {
    let vm = VmConfig::from_cleanup_defaults()?;
    println!("vm-name: {}", vm.vm_name);
    println!("vm-dir: {}", vm.vm_dir().display());
    println!("running: {}", if is_running(&vm)? { "yes" } else { "no" });
    if vm.runtime_file().exists() {
        println!("runtime: {}", vm.runtime_file().display());
        let text = std::fs::read_to_string(vm.runtime_file())?;
        print!("{text}");
    }
    Ok(())
}

pub fn ssh_cmd_cli(cmd: Vec<String>) -> Result<()> {
    let vm = VmConfig::from_cleanup_defaults()?;
    if cmd.is_empty() {
        bail!("ssh: missing remote command");
    }
    let refs: Vec<&str> = cmd.iter().map(String::as_str).collect();
    ssh_cmd(&vm, &refs)
}

// ---------------------------------------------------------------------------
// Internal lifecycle
// ---------------------------------------------------------------------------

fn up(vm: &mut VmConfig) -> Result<()> {
    ensure_dirs(vm)?;
    log(&format!("workspace={}", vm.workspace.display()));
    log(&format!("target={}", vm.target_dir.display()));
    log(&format!("work={}", vm.work_dir.display()));

    if vm.recreate {
        if is_running(vm)? {
            log("recreate requested; stopping existing VM");
            down(vm)?;
        }
        remove_if_exists(&vm.known_hosts())?;
    }

    if is_running(vm)? {
        check_running_mount_paths(vm)?;
        log("vm already running; skipping boot path");
        wait_for_ssh(vm)?;
        log("ensuring /app, /target and /work mounts");
        ensure_guest_mounts(vm)?;
        log(&format!(
            "{} ready (ssh: {}@127.0.0.1:{})",
            vm.vm_name, vm.ssh_user, vm.ssh_port
        ));
        return Ok(());
    }

    ensure_image(vm)?;
    ensure_key(vm)?;
    log("rendering cloud-init");
    render_cloud_init(vm)?;
    create_seed(vm)?;
    ensure_disk(vm)?;
    log("starting qemu");
    start_vm(vm)?;
    wait_for_ssh(vm)?;
    log("ensuring /app, /target and /work mounts");
    ensure_guest_mounts(vm)?;
    log(&format!(
        "{} ready (ssh: {}@127.0.0.1:{})",
        vm.vm_name, vm.ssh_user, vm.ssh_port
    ));
    Ok(())
}

fn down(vm: &VmConfig) -> Result<()> {
    cleanup_seed_server(vm)?;
    if !is_running(vm)? {
        cleanup_virtiofsd(vm)?;
        log(&format!("{} is not running", vm.vm_name));
        return Ok(());
    }

    let pid = read_pid(&vm.pid_file())?.context("missing qemu pid")?;
    kill_pid(pid);
    for _ in 0..20 {
        if !pid_alive(pid) {
            remove_if_exists(&vm.pid_file())?;
            remove_if_exists(&vm.runtime_file())?;
            cleanup_virtiofsd(vm)?;
            log(&format!("{} stopped", vm.vm_name));
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }

    force_kill_pid(pid);
    remove_if_exists(&vm.pid_file())?;
    remove_if_exists(&vm.runtime_file())?;
    cleanup_virtiofsd(vm)?;
    log(&format!("{} stopped (forced)", vm.vm_name));
    Ok(())
}

fn run_in_guest(vm: &VmConfig, args: &RunVmArgs) -> Result<()> {
    let guest_exe =
        ensure_guest_runner_binary(&vm.work_dir, &vm.target_dir, &args.patchbay_version)?;
    let auto_build_overrides = assemble_guest_build_overrides(&vm.target_dir, args)?;
    let staged_overrides = stage_binary_overrides(
        &args.binary_overrides,
        &vm.work_dir,
        &vm.target_dir,
        default_musl_target(),
    )?;

    let mut parts = vec![
        "sudo".to_string(),
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
        parts.push(to_guest_sim_path(&vm.workspace, sim)?);
    }

    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    ssh_cmd(vm, &refs)
}

fn prepare_vm_guest(vm: &VmConfig) -> Result<()> {
    ssh_cmd(vm, &["sudo", "bash", "-lc", GUEST_PREPARE_SCRIPT])
}

// ---------------------------------------------------------------------------
// SSH
// ---------------------------------------------------------------------------

fn ssh_cmd(vm: &VmConfig, remote_args: &[&str]) -> Result<()> {
    ssh_cmd_status(vm, remote_args)
}

fn ssh_cmd_status(vm: &VmConfig, remote_args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-i")
        .arg(vm.ssh_key())
        .args([
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            &format!("UserKnownHostsFile={}", vm.known_hosts().display()),
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "ConnectTimeout=5",
            "-p",
        ])
        .arg(&vm.ssh_port)
        .arg(format!("{}@127.0.0.1", vm.ssh_user));

    if !remote_args.is_empty() {
        let remote = shell_join(remote_args);
        cmd.arg(remote);
    }
    run_checked(&mut cmd, "ssh")
}

fn ssh_probe(vm: &VmConfig) -> bool {
    ssh_probe_inner(vm, false)
}

fn ssh_probe_verbose(vm: &VmConfig) -> bool {
    ssh_probe_inner(vm, true)
}

fn ssh_probe_inner(vm: &VmConfig, verbose: bool) -> bool {
    let mut cmd = Command::new("ssh");
    cmd.arg("-i")
        .arg(vm.ssh_key())
        .args([
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            &format!("UserKnownHostsFile={}", vm.known_hosts().display()),
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            if verbose {
                "LogLevel=VERBOSE"
            } else {
                "LogLevel=ERROR"
            },
            "-o",
            "ConnectTimeout=1",
            "-p",
        ])
        .arg(&vm.ssh_port)
        .arg(format!("{}@127.0.0.1", vm.ssh_user))
        .arg("true")
        .stdout(Stdio::null());
    if verbose {
        cmd.stderr(Stdio::piped());
        match cmd.output() {
            Ok(out) => {
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    for line in stderr.lines() {
                        log(&format!("ssh-probe: {line}"));
                    }
                }
                out.status.success()
            }
            Err(e) => {
                log(&format!("ssh-probe error: {e}"));
                false
            }
        }
    } else {
        cmd.stderr(Stdio::null());
        cmd.status().map(|s| s.success()).unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// VM provisioning helpers
// ---------------------------------------------------------------------------

fn ensure_dirs(vm: &VmConfig) -> Result<()> {
    std::fs::create_dir_all(vm.vm_dir())
        .with_context(|| format!("create {}", vm.vm_dir().display()))?;
    std::fs::create_dir_all(&vm.shared_image_dir)
        .with_context(|| format!("create {}", vm.shared_image_dir.display()))
}

fn persist_runtime(vm: &VmConfig) -> Result<()> {
    let text = format!(
        "workspace={}\ntarget_dir={}\nwork_dir={}\nfs_mode={}\nssh_port={}\n",
        vm.workspace.display(),
        vm.target_dir.display(),
        vm.work_dir.display(),
        vm.fs_mode,
        vm.ssh_port
    );
    std::fs::write(vm.runtime_file(), text)
        .with_context(|| format!("write {}", vm.runtime_file().display()))
}

fn check_running_mount_paths(vm: &VmConfig) -> Result<()> {
    if !vm.runtime_file().exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(vm.runtime_file())
        .with_context(|| format!("read {}", vm.runtime_file().display()))?;
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

    if running_workspace.as_deref() != Some(vm.workspace.to_string_lossy().as_ref()) {
        bail!(
            "VM already running with workspace '{}', requested '{}' (use --recreate)",
            running_workspace.unwrap_or_default(),
            vm.workspace.display()
        );
    }
    if running_target.as_deref() != Some(vm.target_dir.to_string_lossy().as_ref()) {
        bail!(
            "VM already running with target dir '{}', requested '{}' (use --recreate)",
            running_target.unwrap_or_default(),
            vm.target_dir.display()
        );
    }
    if running_work.as_deref() != Some(vm.work_dir.to_string_lossy().as_ref()) {
        bail!(
            "VM already running with work dir '{}', requested '{}' (use --recreate)",
            running_work.unwrap_or_default(),
            vm.work_dir.display()
        );
    }
    Ok(())
}

fn cleanup_seed_server(vm: &VmConfig) -> Result<()> {
    if let Some(pid) = read_pid(&vm.seed_pid_file())? {
        kill_pid(pid);
    }
    remove_if_exists(&vm.seed_pid_file())
}

fn cleanup_virtiofsd(vm: &VmConfig) -> Result<()> {
    for pid_file in [
        vm.workspace_vfs_pid(),
        vm.target_vfs_pid(),
        vm.work_vfs_pid(),
    ] {
        if let Some(pid) = read_pid(&pid_file)? {
            kill_pid(pid);
        }
        remove_if_exists(&pid_file)?;
    }
    remove_if_exists(&vm.workspace_sock())?;
    remove_if_exists(&vm.target_sock())?;
    remove_if_exists(&vm.work_sock())?;
    Ok(())
}

fn detect_virtiofsd_bin(vm: &VmConfig) -> Option<PathBuf> {
    if let Some(bin) = &vm.virtiofsd_bin {
        if bin.exists() {
            return Some(bin.clone());
        }
    }
    for cand in DEFAULT_VIRTIOFSD {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn select_fs_mode(vm: &mut VmConfig) {
    if let Some(bin) = detect_virtiofsd_bin(vm) {
        vm.fs_mode = "virtiofs".to_string();
        vm.virtiofsd_bin = Some(bin);
    } else {
        vm.fs_mode = "9p".to_string();
    }
}

fn is_running(vm: &VmConfig) -> Result<bool> {
    let Some(pid) = read_pid(&vm.pid_file())? else {
        return Ok(false);
    };
    Ok(pid_alive(pid))
}

fn detect_accel(vm: &VmConfig) -> Result<(String, String)> {
    let os = std::env::consts::OS;
    let mut accel = "tcg".to_string();
    let mut cpu = "max".to_string();

    if os == "linux" && Path::new("/dev/kvm").exists() {
        accel = "kvm".to_string();
        cpu = "host".to_string();
    } else if os == "macos" {
        let out = Command::new(&vm.qemu_bin)
            .args(["-accel", "help"])
            .output()
            .with_context(|| format!("run {} -accel help", vm.qemu_bin))?;
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.lines().any(|l| l.trim() == "hvf") {
                accel = "hvf".to_string();
                cpu = "host".to_string();
            }
        }
    }

    Ok((accel, cpu))
}

fn find_aarch64_efi(qemu_bin: &str) -> Option<PathBuf> {
    if let Ok(out) = Command::new("which").arg(qemu_bin).output() {
        if out.status.success() {
            let qemu_path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if let Some(bin_dir) = qemu_path.parent() {
                let candidate = bin_dir.join("../share/qemu/edk2-aarch64-code.fd");
                if let Ok(p) = candidate.canonicalize() {
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    for path in [
        "/opt/homebrew/share/qemu/edk2-aarch64-code.fd",
        "/usr/share/qemu/edk2-aarch64-code.fd",
        "/usr/share/AAVMF/AAVMF_CODE.fd",
        "/usr/share/edk2/aarch64/QEMU_EFI.fd",
    ] {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn ensure_image(vm: &VmConfig) -> Result<()> {
    if vm.base_img().exists() {
        return Ok(());
    }
    log("downloading base image...");
    need_cmd("curl")?;
    let tmp = vm.base_img().with_extension("qcow2.tmp");
    run_checked(
        Command::new("curl")
            .args(["-fsSL", &vm.image_url, "-o"])
            .arg(&tmp),
        "download base image",
    )?;
    std::fs::rename(&tmp, vm.base_img())
        .with_context(|| format!("move {}", vm.base_img().display()))
}

fn ensure_key(vm: &VmConfig) -> Result<()> {
    if vm.ssh_key().exists() && vm.ssh_key().with_extension("pub").exists() {
        return Ok(());
    }
    need_cmd("ssh-keygen")?;
    run_checked(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(vm.ssh_key()),
        "generate ssh key",
    )
}

fn render_cloud_init(vm: &VmConfig) -> Result<()> {
    let pub_key =
        std::fs::read_to_string(vm.ssh_key().with_extension("pub")).context("read ssh pubkey")?;

    let user_data = format!(
        "#cloud-config\nusers:\n  - default\n  - name: {}\n    shell: /bin/bash\n    sudo: ALL=(ALL) NOPASSWD:ALL\n    groups: [sudo]\n    ssh_authorized_keys:\n      - {}\nssh_pwauth: false\nwrite_files:\n  - path: /etc/modules-load.d/patchbay.conf\n    permissions: \"0644\"\n    content: |\n      sch_netem\n      virtiofs\nruncmd:\n  - modprobe sch_netem || true\n  - modprobe virtiofs || true\n  - modprobe 9p || true\n  - modprobe 9pnet_virtio || true\n  - mkdir -p /app /target /work\n",
        vm.ssh_user,
        pub_key.trim()
    );
    std::fs::write(vm.user_data(), user_data)
        .with_context(|| format!("write {}", vm.user_data().display()))?;

    std::fs::write(
        vm.meta_data(),
        format!(
            "instance-id: {}\nlocal-hostname: {}\n",
            vm.vm_name, vm.vm_name
        ),
    )
    .with_context(|| format!("write {}", vm.meta_data().display()))?;

    std::fs::write(
        vm.network_cfg(),
        "version: 2\nethernets:\n  id0:\n    match:\n      driver: virtio_net\n    dhcp4: true\n",
    )
    .with_context(|| format!("write {}", vm.network_cfg().display()))?;

    Ok(())
}

fn create_seed(vm: &VmConfig) -> Result<()> {
    if create_seed_iso(vm)? {
        return Ok(());
    }
    create_seed_http(vm)
}

fn create_seed_iso(vm: &VmConfig) -> Result<bool> {
    if common::command_exists("cloud-localds")? {
        run_checked(
            Command::new("cloud-localds")
                .arg("-N")
                .arg(vm.network_cfg())
                .arg(vm.seed_img())
                .arg(vm.user_data())
                .arg(vm.meta_data()),
            "cloud-localds",
        )?;
        std::fs::write(vm.seed_mode_file(), "iso\n")?;
        return Ok(true);
    }

    let mkiso = if common::command_exists("genisoimage")? {
        Some(("genisoimage", vec![]))
    } else if common::command_exists("mkisofs")? {
        Some(("mkisofs", vec![]))
    } else if common::command_exists("xorriso")? {
        Some(("xorriso", vec!["-as", "mkisofs"]))
    } else {
        None
    };

    let Some((tool, mut prefix_args)) = mkiso else {
        return Ok(false);
    };

    let tmp = vm.vm_dir().join(format!("seed.{}", std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).with_context(|| format!("remove {}", tmp.display()))?;
    }
    std::fs::create_dir_all(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    std::fs::copy(vm.user_data(), tmp.join("user-data"))?;
    std::fs::copy(vm.meta_data(), tmp.join("meta-data"))?;
    std::fs::copy(vm.network_cfg(), tmp.join("network-config"))?;

    let mut cmd = Command::new(tool);
    for a in prefix_args.drain(..) {
        cmd.arg(a);
    }
    run_checked(
        cmd.args(["-output"])
            .arg(vm.seed_img())
            .args(["-volid", "cidata", "-joliet", "-rock"])
            .arg(&tmp),
        "make seed iso",
    )?;
    std::fs::remove_dir_all(&tmp).ok();
    std::fs::write(vm.seed_mode_file(), "iso\n")?;
    Ok(true)
}

fn create_seed_http(vm: &VmConfig) -> Result<()> {
    std::fs::create_dir_all(vm.seed_dir())
        .with_context(|| format!("create {}", vm.seed_dir().display()))?;
    std::fs::copy(vm.user_data(), vm.seed_dir().join("user-data"))?;
    std::fs::copy(vm.meta_data(), vm.seed_dir().join("meta-data"))?;
    std::fs::copy(vm.network_cfg(), vm.seed_dir().join("network-config"))?;
    std::fs::write(vm.seed_mode_file(), "http\n")?;
    Ok(())
}

fn start_seed_server(vm: &VmConfig) -> Result<()> {
    let mode = std::fs::read_to_string(vm.seed_mode_file()).unwrap_or_default();
    if mode.trim() != "http" {
        return Ok(());
    }

    cleanup_seed_server(vm)?;
    need_cmd("python3")?;
    let log_file = File::create(vm.p("seed-http.log"))
        .with_context(|| format!("create {}", vm.p("seed-http.log").display()))?;
    let log2 = log_file.try_clone().context("clone seed log")?;

    let child = Command::new("python3")
        .args(["-m", "http.server", &vm.seed_port, "--bind", "0.0.0.0"])
        .current_dir(vm.seed_dir())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log2))
        .spawn()
        .context("start cloud-init seed http server")?;

    std::fs::write(vm.seed_pid_file(), format!("{}\n", child.id()))?;
    thread::sleep(Duration::from_secs(1));
    if !pid_alive(child.id() as i32) {
        bail!(
            "cloud-init HTTP seed server failed to start on port {}",
            vm.seed_port
        );
    }
    Ok(())
}

fn start_virtiofsd(vm: &VmConfig) -> Result<()> {
    if vm.fs_mode != "virtiofs" {
        return Ok(());
    }
    cleanup_virtiofsd(vm)?;
    let virtiofsd = vm
        .virtiofsd_bin
        .as_ref()
        .context("virtiofs mode selected but virtiofsd missing")?;

    spawn_virtiofsd(
        virtiofsd,
        &vm.workspace,
        &vm.workspace_sock(),
        &vm.p("workspace.virtiofsd.log"),
        &vm.workspace_vfs_pid(),
        true,
    )?;
    spawn_virtiofsd(
        virtiofsd,
        &vm.target_dir,
        &vm.target_sock(),
        &vm.p("target.virtiofsd.log"),
        &vm.target_vfs_pid(),
        true,
    )?;
    spawn_virtiofsd(
        virtiofsd,
        &vm.work_dir,
        &vm.work_sock(),
        &vm.p("work.virtiofsd.log"),
        &vm.work_vfs_pid(),
        false,
    )?;

    for _ in 0..30 {
        if vm.workspace_sock().exists() && vm.target_sock().exists() && vm.work_sock().exists() {
            let wp = read_pid(&vm.workspace_vfs_pid())?;
            let tp = read_pid(&vm.target_vfs_pid())?;
            let wk = read_pid(&vm.work_vfs_pid())?;
            if let (Some(wp), Some(tp), Some(wk)) = (wp, tp, wk) {
                if pid_alive(wp) && pid_alive(tp) && pid_alive(wk) {
                    return Ok(());
                }
            }
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!(
        "virtiofsd failed to become healthy; check {}/workspace.virtiofsd.log, {}/target.virtiofsd.log and {}/work.virtiofsd.log",
        vm.vm_dir().display(),
        vm.vm_dir().display(),
        vm.vm_dir().display()
    );
}

fn spawn_virtiofsd(
    bin: &Path,
    shared_dir: &Path,
    socket_path: &Path,
    log_path: &Path,
    pid_path: &Path,
    readonly: bool,
) -> Result<()> {
    let log_file =
        File::create(log_path).with_context(|| format!("create {}", log_path.display()))?;
    let log2 = log_file.try_clone().context("clone virtiofsd log")?;

    let mut cmd = Command::new(bin);
    cmd.arg("--shared-dir")
        .arg(shared_dir)
        .arg("--socket-path")
        .arg(socket_path)
        .args([
            "--cache",
            "auto",
            "--sandbox",
            "none",
            "--inode-file-handles=never",
        ]);
    if readonly {
        cmd.arg("--readonly");
    } else {
        #[cfg(unix)]
        let (uid, gid) = {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::metadata(shared_dir).context("stat shared_dir")?;
            (m.uid(), m.gid())
        };
        #[cfg(not(unix))]
        let (uid, gid) = (1000u32, 1000u32);
        cmd.args([
            "--translate-uid",
            &format!("squash-guest:0:{uid}:65536"),
            "--translate-gid",
            &format!("squash-guest:0:{gid}:65536"),
        ]);
    }
    let child = cmd
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log2))
        .spawn()
        .with_context(|| format!("start {}", bin.display()))?;

    std::fs::write(pid_path, format!("{}\n", child.id()))
        .with_context(|| format!("write {}", pid_path.display()))
}

fn ensure_disk(vm: &VmConfig) -> Result<()> {
    need_cmd("qemu-img")?;
    if vm.disk_img().exists() {
        return Ok(());
    }
    run_checked(
        Command::new("qemu-img")
            .args(["create", "-f", "qcow2", "-F", "qcow2", "-b"])
            .arg(vm.base_img())
            .arg(vm.disk_img())
            .arg(format!("{}G", vm.disk_gb)),
        "qemu-img create",
    )
}

fn wait_for_ssh(vm: &VmConfig) -> Result<()> {
    log(&format!("waiting for SSH on 127.0.0.1:{} ...", vm.ssh_port));
    log(&format!(
        "ssh key: {} (exists={})",
        vm.ssh_key().display(),
        vm.ssh_key().exists()
    ));
    let mut last_msg = String::new();
    for i in 1..=180 {
        let ok = if i % 30 == 0 {
            ssh_probe_verbose(vm)
        } else {
            ssh_probe(vm)
        };
        if ok {
            cleanup_seed_server(vm)?;
            log("SSH is reachable");
            return Ok(());
        }
        if i % 5 == 0 && vm.serial_log().exists() {
            if let Ok(text) = std::fs::read_to_string(vm.serial_log()) {
                if let Some(last) = text.lines().last() {
                    let line = last.trim_end_matches('\r').to_string();
                    if line != last_msg {
                        log(&format!("booting... {line}"));
                        last_msg = line;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(300));
    }
    cleanup_seed_server(vm)?;
    bail!(
        "VM did not become reachable via SSH on port {} (try --recreate)",
        vm.ssh_port
    )
}

fn ensure_guest_mounts(vm: &VmConfig) -> Result<()> {
    let mnt_opts = "trans=virtio,version=9p2000.L,msize=262144";
    ssh_cmd(vm, &["sudo", "mkdir", "-p", "/app", "/target", "/work"])?;
    ssh_cmd(
        vm,
        &[
            "sudo",
            "sh",
            "-lc",
            "sed -i '/[[:space:]]\\/app[[:space:]].*9p/d; /[[:space:]]\\/target[[:space:]].*9p/d; /[[:space:]]\\/work[[:space:]].*9p/d' /etc/fstab || true",
        ],
    )?;

    if vm.fs_mode == "virtiofs" {
        ssh_cmd(
            vm,
            &[
                "sudo",
                "sh",
                "-lc",
                &format!("mountpoint -q /app || mount -t virtiofs -o ro workspace /app || mount -t 9p -o {mnt_opts},ro workspace /app"),
            ],
        )?;
        ssh_cmd(
            vm,
            &[
                "sudo",
                "sh",
                "-lc",
                &format!("mountpoint -q /target || mount -t virtiofs -o ro target /target || mount -t 9p -o {mnt_opts},ro target /target"),
            ],
        )?;
        ssh_cmd(
            vm,
            &[
                "sudo",
                "sh",
                "-lc",
                &format!("mountpoint -q /work || mount -t virtiofs work /work || mount -t 9p -o {mnt_opts} work /work"),
            ],
        )?;
    } else {
        ssh_cmd(
            vm,
            &[
                "sudo",
                "sh",
                "-lc",
                &format!("mountpoint -q /app || mount -t 9p -o {mnt_opts},ro workspace /app || mount -t virtiofs -o ro workspace /app"),
            ],
        )?;
        ssh_cmd(
            vm,
            &[
                "sudo",
                "sh",
                "-lc",
                &format!("mountpoint -q /target || mount -t 9p -o {mnt_opts},ro target /target || mount -t virtiofs -o ro target /target"),
            ],
        )?;
        ssh_cmd(
            vm,
            &[
                "sudo",
                "sh",
                "-lc",
                &format!("mountpoint -q /work || mount -t 9p -o {mnt_opts} work /work || mount -t virtiofs work /work"),
            ],
        )?;
    }

    ssh_cmd(vm, &["sudo", "mount", "-o", "remount,ro", "/app"])?;
    ssh_cmd(vm, &["sudo", "mount", "-o", "remount,ro", "/target"])?;
    ssh_cmd(vm, &["sudo", "mount", "-o", "remount,rw", "/work"])?;

    ssh_cmd(vm, &["test", "-f", "/app/Cargo.toml"])
        .context("/app is mounted but missing /app/Cargo.toml")?;
    ssh_cmd(vm, &["test", "-d", "/target"]).context("/target mount is unavailable")?;
    ssh_cmd(vm, &["test", "-d", "/work"]).context("/work mount is unavailable")?;
    Ok(())
}

fn start_vm(vm: &mut VmConfig) -> Result<()> {
    if is_running(vm)? {
        return Ok(());
    }

    ensure_ssh_port_available(vm)?;
    need_cmd(&vm.qemu_bin)?;
    need_cmd("ssh")?;
    std::fs::create_dir_all(&vm.target_dir)?;
    std::fs::create_dir_all(&vm.work_dir)?;

    select_fs_mode(vm);
    if vm.fs_mode == "virtiofs" {
        start_virtiofsd(vm)?;
    }
    start_seed_server(vm)?;

    let (accel, cpu) = detect_accel(vm)?;
    let seed_mode = std::fs::read_to_string(vm.seed_mode_file()).unwrap_or_default();
    let is_aarch64 = vm.qemu_bin.contains("aarch64");

    let mut qemu = Command::new(&vm.qemu_bin);
    qemu.arg("-name")
        .arg(&vm.vm_name)
        .arg("-daemonize")
        .arg("-pidfile")
        .arg(vm.pid_file())
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg(format!("file:{}", vm.serial_log().display()))
        .arg("-m")
        .arg(&vm.mem_mb)
        .arg("-smp")
        .arg(&vm.cpus)
        .arg("-accel")
        .arg(accel)
        .arg("-cpu")
        .arg(cpu);

    if is_aarch64 {
        qemu.arg("-M").arg("virt");
        if let Some(efi) = find_aarch64_efi(&vm.qemu_bin) {
            qemu.arg("-bios").arg(efi);
        }
    }

    qemu.arg("-drive").arg(format!(
        "if=virtio,format=qcow2,file={}",
        vm.disk_img().display()
    ));

    if seed_mode.trim() == "iso" {
        qemu.arg("-drive").arg(format!(
            "if=virtio,media=cdrom,format=raw,readonly=on,file={}",
            vm.seed_img().display()
        ));
    } else {
        qemu.arg("-smbios").arg(format!(
            "type=1,serial=ds=nocloud;s=http://10.0.2.2:{}/",
            vm.seed_port
        ));
    }

    qemu.arg("-netdev")
        .arg(format!(
            "user,id=net0,hostfwd=tcp:127.0.0.1:{}-:22",
            vm.ssh_port
        ))
        .arg("-device")
        .arg("virtio-net-pci,netdev=net0");

    if vm.fs_mode == "virtiofs" {
        qemu.arg("-object")
            .arg(format!(
                "memory-backend-memfd,id=mem,size={}M,share=on",
                vm.mem_mb
            ))
            .arg("-numa")
            .arg("node,memdev=mem")
            .arg("-chardev")
            .arg(format!(
                "socket,id=workspacefs,path={}",
                vm.workspace_sock().display()
            ))
            .arg("-device")
            .arg("vhost-user-fs-pci,chardev=workspacefs,tag=workspace")
            .arg("-chardev")
            .arg(format!(
                "socket,id=targetfs,path={}",
                vm.target_sock().display()
            ))
            .arg("-device")
            .arg("vhost-user-fs-pci,chardev=targetfs,tag=target")
            .arg("-chardev")
            .arg(format!(
                "socket,id=workfs,path={}",
                vm.work_sock().display()
            ))
            .arg("-device")
            .arg("vhost-user-fs-pci,chardev=workfs,tag=work");
    } else {
        qemu.arg("-virtfs").arg(format!(
            "local,path={},mount_tag=workspace,security_model=none,multidevs=remap,id=workspace,readonly=on",
            vm.workspace.display()
        ));
        qemu.arg("-virtfs").arg(format!(
            "local,path={},mount_tag=target,security_model=none,multidevs=remap,id=target,readonly=on",
            vm.target_dir.display()
        ));
        qemu.arg("-virtfs").arg(format!(
            "local,path={},mount_tag=work,security_model=none,multidevs=remap,id=work",
            vm.work_dir.display()
        ));
    }

    run_checked(&mut qemu, "start qemu")?;
    persist_runtime(vm)
}

fn ensure_ssh_port_available(vm: &VmConfig) -> Result<()> {
    let addr = format!("127.0.0.1:{}", vm.ssh_port);
    match TcpListener::bind(&addr) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(err) => bail!(
            "SSH forward port {} is already in use ({err}). Stop the conflicting VM/process or set QEMU_VM_SSH_PORT to a free port (for example: QEMU_VM_SSH_PORT=2223).",
            vm.ssh_port
        ),
    }
}

fn shared_image_dir() -> Result<PathBuf> {
    if let Some(data) = dirs::data_dir() {
        return Ok(data.join("patchbay").join("qemu-images"));
    }
    let home = dirs::home_dir().context("resolve home dir for shared image cache")?;
    Ok(home.join(".local/share/patchbay/qemu-images"))
}

fn base_image_name(image_url: &str) -> String {
    let tail = image_url
        .rsplit('/')
        .next()
        .unwrap_or("base-image")
        .split('?')
        .next()
        .unwrap_or("base-image");
    let tail = tail.strip_suffix(".qcow2").unwrap_or(tail);
    let clean = sanitize_filename(tail);
    let hash = fnv1a64(image_url.as_bytes());
    format!("{clean}-{hash:016x}.qcow2")
}
