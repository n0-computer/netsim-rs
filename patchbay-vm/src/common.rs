use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, bail, Context, Result};
use patchbay_utils::{
    assets::{infer_binary_mode, parse_binary_overrides, BinarySpec},
    binary_cache::set_executable,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

const RELEASE_MUSL_ASSET_X86: &str = "patchbay-x86_64-unknown-linux-musl.tar.gz";
const RELEASE_MUSL_ASSET_ARM64: &str = "patchbay-aarch64-unknown-linux-musl.tar.gz";
const GITHUB_REPO: &str = "https://github.com/n0-computer/patchbay.git";
const DEFAULT_MUSL_TARGET_X86: &str = "x86_64-unknown-linux-musl";
const DEFAULT_MUSL_TARGET_ARM64: &str = "aarch64-unknown-linux-musl";

pub const DEFAULT_MEM_MB: &str = "8192";

/// Returns the number of logical CPUs as a string, for use as the default guest CPU count.
pub fn default_cpus() -> String {
    std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "4".to_string())
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RunVmArgs {
    pub sim_inputs: Vec<PathBuf>,
    pub work_dir: PathBuf,
    pub binary_overrides: Vec<String>,
    pub verbose: bool,
    pub recreate: bool,
    pub patchbay_version: String,
}

#[derive(Debug, Clone)]
pub struct TestVmArgs {
    pub filter: Option<String>,
    pub target: String,
    pub packages: Vec<String>,
    pub tests: Vec<String>,
    pub recreate: bool,
    pub cargo_args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VmExtends {
    pub file: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VmSimMeta {
    pub binaries: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VmSimFile {
    #[serde(default)]
    pub sim: VmSimMeta,
    #[serde(default)]
    pub extends: Vec<VmExtends>,
    #[serde(default, rename = "binary")]
    pub binaries: Vec<BinarySpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmBuildRequest {
    pub source_dir: PathBuf,
    pub example: Option<String>,
    pub bin: Option<String>,
    pub features: Vec<String>,
    pub all_features: bool,
}

// ---------------------------------------------------------------------------
// Host arch helpers
// ---------------------------------------------------------------------------

pub fn is_arm64_host() -> bool {
    std::env::consts::ARCH == "aarch64"
}

pub fn default_musl_target() -> &'static str {
    if is_arm64_host() {
        DEFAULT_MUSL_TARGET_ARM64
    } else {
        DEFAULT_MUSL_TARGET_X86
    }
}

pub fn release_musl_asset() -> &'static str {
    if is_arm64_host() {
        RELEASE_MUSL_ASSET_ARM64
    } else {
        RELEASE_MUSL_ASSET_X86
    }
}

// ---------------------------------------------------------------------------
// Build / staging helpers
// ---------------------------------------------------------------------------

/// Resolve and stage the patchbay runner binary for use inside the guest.
pub fn ensure_guest_runner_binary(
    work_dir: &Path,
    target_dir: &Path,
    version: &str,
) -> Result<String> {
    let source = resolve_vm_runner_binary(work_dir, target_dir, version)?;
    let staged_dir = work_dir.join(".patchbay-bin");
    std::fs::create_dir_all(&staged_dir)
        .with_context(|| format!("create {}", staged_dir.display()))?;
    let staged = staged_dir.join("patchbay");
    std::fs::copy(&source, &staged)
        .with_context(|| format!("copy {} -> {}", source.display(), staged.display()))?;
    set_executable(&staged)?;
    Ok("/work/.patchbay-bin/patchbay".to_string())
}

fn resolve_vm_runner_binary(
    work_dir: &Path,
    _target_dir: &Path,
    version: &str,
) -> Result<PathBuf> {
    match std::env::consts::OS {
        "linux" | "macos" => {
            if let Some(path) = version.strip_prefix("path:") {
                let bin = PathBuf::from(path);
                if !bin.exists() {
                    bail!("--patchbay-version path does not exist: {}", bin.display());
                }
                if bin.is_dir() {
                    bail!(
                        "--patchbay-version path points to a directory, expected executable file: {}",
                        bin.display()
                    );
                }
                return Ok(bin);
            }
            if let Some(git_ref) = version.strip_prefix("git:") {
                build_musl_from_git_ref(work_dir, git_ref)
            } else {
                download_release_runner(work_dir, version)
            }
        }
        other => bail!("run-vm is not supported on host OS '{}'", other),
    }
}

fn download_release_runner(work_dir: &Path, version: &str) -> Result<PathBuf> {
    need_cmd("curl")?;
    need_cmd("tar")?;
    let cache_root = work_dir.join(".vm-cache");
    std::fs::create_dir_all(&cache_root)
        .with_context(|| format!("create {}", cache_root.display()))?;
    let version_key = if version == "latest" {
        "latest".to_string()
    } else {
        normalize_release_tag(version)
    };
    let archive = cache_root.join(format!(
        "{}-{}",
        version_key.replace('/', "_"),
        release_musl_asset()
    ));
    let unpack = cache_root.join(format!(
        "release-{}-{}",
        version_key.replace('/', "_"),
        default_musl_target()
    ));
    let cached_bin = unpack.join("patchbay");
    if cached_bin.exists() {
        return Ok(cached_bin);
    }

    let url = if version == "latest" {
        format!(
            "https://github.com/n0-computer/patchbay/releases/latest/download/{}",
            release_musl_asset()
        )
    } else {
        format!(
            "https://github.com/n0-computer/patchbay/releases/download/{}/{}",
            normalize_release_tag(version),
            release_musl_asset()
        )
    };

    run_checked(
        Command::new("curl").args(["-fL", &url, "-o"]).arg(&archive),
        "download patchbay musl release",
    )?;

    if unpack.exists() {
        std::fs::remove_dir_all(&unpack).with_context(|| format!("remove {}", unpack.display()))?;
    }
    std::fs::create_dir_all(&unpack).with_context(|| format!("create {}", unpack.display()))?;
    run_checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpack),
        "extract patchbay musl release",
    )?;

    let bin = find_file_named(&unpack, "patchbay")
        .with_context(|| format!("find patchbay binary under {}", unpack.display()))?;
    set_executable(&bin)?;
    Ok(bin)
}

fn build_musl_from_git_ref(work_dir: &Path, git_ref: &str) -> Result<PathBuf> {
    let checkout_root = work_dir.join(".vm-cache").join("git");
    std::fs::create_dir_all(&checkout_root)
        .with_context(|| format!("create {}", checkout_root.display()))?;
    let checkout = checkout_root.join("patchbay");

    if !checkout.exists() {
        run_checked(
            Command::new("git")
                .args(["clone", "--no-tags", GITHUB_REPO])
                .arg(&checkout),
            "clone patchbay repo",
        )?;
    }

    run_checked(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["fetch", "--all", "--prune"]),
        "git fetch patchbay repo",
    )?;
    run_checked(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["checkout", git_ref]),
        "git checkout requested ref",
    )?;

    let target_dir = work_dir.join(".vm-cache").join("git-target");
    std::fs::create_dir_all(&target_dir)?;
    run_checked(
        Command::new("cargo")
            .args([
                "build",
                "--release",
                "--target",
                default_musl_target(),
                "--bin",
                "patchbay",
            ])
            .env("CARGO_TARGET_DIR", &target_dir)
            .current_dir(&checkout),
        "build patchbay from git ref",
    )?;
    let bin = target_dir
        .join(default_musl_target())
        .join("release")
        .join("patchbay");
    if !bin.exists() {
        bail!("built patchbay binary missing at {}", bin.display());
    }
    Ok(bin)
}

/// Assemble `--binary` overrides for binaries that need building on the host.
pub fn assemble_guest_build_overrides(
    target_dir: &Path,
    args: &RunVmArgs,
) -> Result<Vec<String>> {
    let user_override_names = parse_binary_overrides(&args.binary_overrides)?
        .into_keys()
        .collect::<std::collections::HashSet<_>>();
    let sim_files = expand_vm_sim_inputs(&args.sim_inputs)?;
    let mut requested: HashMap<String, VmBuildRequest> = HashMap::new();
    let mut first_seen: HashMap<String, PathBuf> = HashMap::new();

    for sim_path in sim_files {
        let (sim, sim_root) = load_vm_sim(&sim_path)?;
        let merged = merged_vm_binary_specs(&sim, &sim_path)?;
        for spec in merged.into_values() {
            if user_override_names.contains(&spec.name) {
                continue;
            }
            if infer_binary_mode(&spec)? != "build" {
                continue;
            }
            if spec.repo.is_some() {
                bail!(
                    "VM auto-override does not support repo-based build spec '{}' in {}",
                    spec.name,
                    sim_path.display()
                );
            }
            let source_dir = resolve_vm_build_source_dir(&spec, &sim_root)?;
            let req = VmBuildRequest {
                source_dir,
                example: spec.example.clone(),
                bin: spec.bin.clone(),
                features: spec.features.clone(),
                all_features: spec.all_features,
            };
            if let Some(existing) = requested.get(&spec.name) {
                if existing != &req {
                    let first = first_seen
                        .get(&spec.name)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    bail!(
                        "duplicate build spec '{}' differs across sims: {} vs {}",
                        spec.name,
                        first,
                        sim_path.display()
                    );
                }
                continue;
            }
            first_seen.insert(spec.name.clone(), sim_path.clone());
            requested.insert(spec.name.clone(), req);
        }
    }

    let mut names: Vec<String> = requested.keys().cloned().collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let req = requested
            .get(&name)
            .ok_or_else(|| anyhow!("missing build request for '{}'", name))?;
        let guest_path = build_vm_binary_and_guest_path(target_dir, &name, req)?;
        out.push(format!("{name}:path:{guest_path}"));
    }
    Ok(out)
}

pub fn expand_vm_sim_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut sims = Vec::new();
    for input in inputs {
        if input.is_file() {
            if input.extension().and_then(|s| s.to_str()) == Some("toml") {
                sims.push(input.clone());
            }
            continue;
        }
        if input.is_dir() {
            for entry in std::fs::read_dir(input)
                .with_context(|| format!("read sim dir {}", input.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("toml") {
                    sims.push(path);
                }
            }
            continue;
        }
        bail!("sim input path does not exist: {}", input.display());
    }
    sims.sort();
    sims.dedup();
    Ok(sims)
}

pub fn load_vm_sim(sim_path: &Path) -> Result<(VmSimFile, PathBuf)> {
    let text = std::fs::read_to_string(sim_path)
        .with_context(|| format!("read sim {}", sim_path.display()))?;
    let sim: VmSimFile =
        toml::from_str(&text).with_context(|| format!("parse sim {}", sim_path.display()))?;
    let root = find_ancestor_with_file(sim_path, "Cargo.toml")
        .unwrap_or_else(|| sim_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    Ok((sim, root))
}

fn merged_vm_binary_specs(sim: &VmSimFile, sim_path: &Path) -> Result<HashMap<String, BinarySpec>> {
    let mut merged = HashMap::new();
    for spec in load_vm_extends_binaries(sim, sim_path)?
        .into_iter()
        .chain(load_vm_shared_binaries(sim, sim_path)?)
        .chain(sim.binaries.clone())
    {
        merged.insert(spec.name.clone(), spec);
    }
    Ok(merged)
}

fn load_vm_extends_binaries(sim: &VmSimFile, sim_path: &Path) -> Result<Vec<BinarySpec>> {
    let sim_dir = sim_path.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for ext in &sim.extends {
        let path = sim_dir.join(&ext.file);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read extends file {}", path.display()))?;
        let parsed: VmSimFile = toml::from_str(&text)
            .with_context(|| format!("parse extends file {}", path.display()))?;
        out.extend(parsed.binaries);
    }
    Ok(out)
}

fn load_vm_shared_binaries(sim: &VmSimFile, sim_path: &Path) -> Result<Vec<BinarySpec>> {
    #[derive(Deserialize, Default)]
    struct BinaryFile {
        #[serde(default, rename = "binary")]
        binaries: Vec<BinarySpec>,
    }

    let Some(ref_name) = sim.sim.binaries.as_deref() else {
        return Ok(vec![]);
    };
    let sim_dir = sim_path.parent().unwrap_or(Path::new("."));
    let path = sim_dir.join(ref_name);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read shared binaries file {}", path.display()))?;
    let parsed: BinaryFile = toml::from_str(&text).context("parse shared binaries file")?;
    Ok(parsed.binaries)
}

fn resolve_vm_build_source_dir(spec: &BinarySpec, default_root: &Path) -> Result<PathBuf> {
    if let Some(path) = &spec.path {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            default_root.join(path)
        };
        if resolved.is_file() {
            bail!(
                "binary '{}' mode=build path must be a directory, got file {}",
                spec.name,
                resolved.display()
            );
        }
        return Ok(resolved);
    }
    Ok(default_root.to_path_buf())
}

fn build_vm_binary_and_guest_path(
    target_dir: &Path,
    name: &str,
    req: &VmBuildRequest,
) -> Result<String> {
    let mut base_args: Vec<String> = vec![
        "build".into(),
        "--release".into(),
        "--target".into(),
        default_musl_target().into(),
    ];
    if req.all_features {
        base_args.push("--all-features".into());
    } else if !req.features.is_empty() {
        base_args.push("--features".into());
        base_args.push(req.features.join(","));
    }

    if let Some(example) = req.example.as_deref() {
        let mut args = base_args.clone();
        args.push("--example".into());
        args.push(example.to_string());
        run_checked(
            Command::new("cargo")
                .args(args)
                .env("CARGO_TARGET_DIR", target_dir)
                .current_dir(&req.source_dir),
            "build VM example binary",
        )?;
        return Ok(format!(
            "/target/{}/release/examples/{}",
            default_musl_target(),
            example
        ));
    }

    if let Some(bin) = req.bin.as_deref() {
        let mut args = base_args;
        args.push("--bin".into());
        args.push(bin.to_string());
        run_checked(
            Command::new("cargo")
                .args(args)
                .env("CARGO_TARGET_DIR", target_dir)
                .current_dir(&req.source_dir),
            "build VM bin binary",
        )?;
        return Ok(format!("/target/{}/release/{}", default_musl_target(), bin));
    }

    let mut example_args = base_args.clone();
    example_args.push("--example".into());
    example_args.push(name.to_string());
    let example_status = Command::new("cargo")
        .args(example_args)
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(&req.source_dir)
        .status()
        .context("spawn cargo build --example for VM")?;
    if example_status.success() {
        return Ok(format!(
            "/target/{}/release/examples/{}",
            default_musl_target(),
            name
        ));
    }

    let mut bin_args = base_args;
    bin_args.push("--bin".into());
    bin_args.push(name.to_string());
    run_checked(
        Command::new("cargo")
            .args(bin_args)
            .env("CARGO_TARGET_DIR", target_dir)
            .current_dir(&req.source_dir),
        "build VM fallback bin",
    )?;
    Ok(format!(
        "/target/{}/release/{}",
        default_musl_target(),
        name
    ))
}

/// Build test binaries on the host and return their paths.
pub fn build_and_collect_test_binaries(
    target_dir: &Path,
    target: &str,
    packages: &[String],
    tests: &[String],
    cargo_args: &[String],
) -> Result<Vec<PathBuf>> {
    use std::io::{BufRead, BufReader};

    let mut cmd = Command::new("cargo");
    cmd.args([
        "test",
        "--no-run",
        "--target",
        target,
        "--message-format",
        "json",
    ]);
    for pkg in packages {
        cmd.args(["-p", pkg]);
    }
    for test in tests {
        cmd.args(["--test", test]);
    }
    if !cargo_args.is_empty() {
        cmd.args(cargo_args);
    }
    cmd.env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_TERM_COLOR", "always");
    eprintln!("[cargo] {cmd:?}");
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawn cargo test --no-run")?;

    let stderr = child.stderr.take().unwrap();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            eprintln!("[cargo] {line}");
        }
    });

    let stdout = child.stdout.take().unwrap();
    let mut stdout_lines = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        stdout_lines.push(line);
    }

    let status = child.wait().context("wait cargo test --no-run")?;
    let _ = stderr_thread.join();
    if !status.success() {
        for line in &stdout_lines {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("reason").and_then(|x| x.as_str()) == Some("compiler-message") {
                if let Some(rendered) = v
                    .get("message")
                    .and_then(|m| m.get("rendered"))
                    .and_then(|r| r.as_str())
                {
                    eprint!("{rendered}");
                }
            }
        }
        bail!("cargo test --no-run failed");
    }

    let mut bins = Vec::new();
    for line in &stdout_lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("reason").and_then(|x| x.as_str()) != Some("compiler-artifact") {
            continue;
        }
        if !v
            .get("profile")
            .and_then(|p| p.get("test"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        let Some(exe) = v.get("executable").and_then(|e| e.as_str()) else {
            continue;
        };
        let path = PathBuf::from(exe);
        if path.exists() && path.is_file() {
            bins.push(path);
        }
    }
    bins.sort();
    bins.dedup();
    Ok(bins)
}

/// Copy test binaries into the work directory and return their guest paths.
pub fn stage_test_binaries(work_dir: &Path, bins: &[PathBuf]) -> Result<Vec<String>> {
    let stage_dir = work_dir.join("binaries").join("tests");
    std::fs::create_dir_all(&stage_dir)
        .with_context(|| format!("create {}", stage_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stage_dir, std::fs::Permissions::from_mode(0o777))?;
    }
    let mut staged_guest = Vec::new();
    for bin in bins {
        let file = bin
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("bad test binary name: {}", bin.display()))?;
        let dest = stage_dir.join(file);
        std::fs::copy(bin, &dest)
            .with_context(|| format!("copy {} -> {}", bin.display(), dest.display()))?;
        set_executable(&dest)?;
        staged_guest.push(format!("/work/binaries/tests/{file}"));
    }
    Ok(staged_guest)
}

// ---------------------------------------------------------------------------
// Pure utility functions
// ---------------------------------------------------------------------------

pub fn log_msg(prefix: &str, msg: &str) {
    eprintln!("[{prefix}] {msg}");
}

pub fn run_checked(cmd: &mut Command, label: &str) -> Result<()> {
    let status = cmd.status().with_context(|| format!("run '{label}'"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("command failed: {label} (status {status})")
    }
}

pub fn need_cmd(name: &str) -> Result<()> {
    if command_exists(name)? {
        Ok(())
    } else {
        bail!("missing required command: {name}")
    }
}

pub fn command_exists(name: &str) -> Result<bool> {
    Ok(Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .status()
        .context("check command")?
        .success())
}

pub fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

pub fn cargo_target_dir() -> Result<PathBuf> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .context("run cargo metadata for target dir")?;
    if !out.status.success() {
        bail!("cargo metadata failed while resolving target dir");
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parse cargo metadata json")?;
    let dir = v
        .get("target_directory")
        .and_then(|s| s.as_str())
        .context("cargo metadata missing target_directory")?;
    Ok(PathBuf::from(dir))
}

pub fn remove_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("remove {}", path.display()))
    } else {
        std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
    }
}

pub fn read_pid(path: &Path) -> Result<Option<i32>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(text.trim().parse::<i32>().ok())
}

pub fn pid_alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 is side-effect free and used only for liveness probing.
    let rc = unsafe { nix::libc::kill(pid, 0) };
    if rc == 0 {
        true
    } else {
        let errno = nix::errno::Errno::last_raw();
        errno == nix::libc::EPERM
    }
}

pub fn kill_pid(pid: i32) {
    // SAFETY: best-effort process signal for known pid.
    let _ = unsafe { nix::libc::kill(pid, nix::libc::SIGTERM) };
}

pub fn force_kill_pid(pid: i32) {
    // SAFETY: best-effort forced process signal for known pid.
    let _ = unsafe { nix::libc::kill(pid, nix::libc::SIGKILL) };
}

pub fn abspath(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub fn to_guest_sim_path(workspace: &Path, sim: &Path) -> Result<String> {
    let sim_abs = if sim.is_absolute() {
        sim.to_path_buf()
    } else {
        std::env::current_dir()?.join(sim)
    };
    let rel = sim_abs.strip_prefix(workspace).with_context(|| {
        format!(
            "sim path {} must be under workspace {}",
            sim_abs.display(),
            workspace.display()
        )
    })?;
    Ok(format!("/app/{}", rel.display()))
}

pub fn shell_join<T: AsRef<str>>(parts: &[T]) -> String {
    parts
        .iter()
        .map(|p| shell_escape(p.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.bytes().all(|b| {
        matches!(
            b,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':'
        )
    }) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

pub fn find_file_named(root: &Path, file_name: &str) -> Result<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for ent in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let ent = ent?;
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|s| s.to_str()) == Some(file_name) {
                return Ok(path);
            }
        }
    }
    bail!("file '{}' not found under {}", file_name, root.display())
}

pub fn find_ancestor_with_file(path: &Path, file_name: &str) -> Option<PathBuf> {
    let mut cur = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if cur.join(file_name).is_file() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

pub fn normalize_release_tag(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

pub fn sanitize_filename(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "base-image".to_string()
    } else {
        out
    }
}

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// The guest package installation script shared by both backends.
pub const GUEST_PREPARE_SCRIPT: &str = "set -euo pipefail; export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH; export DEBIAN_FRONTEND=noninteractive; if ! command -v ip >/dev/null 2>&1 || ! command -v tc >/dev/null 2>&1 || ! command -v nft >/dev/null 2>&1 || ! command -v modprobe >/dev/null 2>&1 || ! command -v sysctl >/dev/null 2>&1; then apt-get update; apt-get install -y bridge-utils iproute2 iputils-ping iptables nftables net-tools curl iperf3 jq kmod procps; fi; modprobe sch_netem || true; sysctl -w net.ipv4.ip_forward=1";
