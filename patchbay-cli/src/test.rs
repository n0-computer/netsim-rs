//! Test command implementation.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use patchbay_utils::manifest::{self, RunKind, RunManifest, TestStatus};

/// Check if cargo-nextest is available.
fn has_nextest() -> bool {
    Command::new("cargo-nextest")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Shared test arguments used by both `patchbay test` and `patchbay compare test`.
#[derive(Debug, Clone, clap::Args)]
pub struct TestArgs {
    /// Include ignored tests (like `cargo test -- --include-ignored`).
    #[arg(long)]
    pub include_ignored: bool,

    /// Run only ignored tests (like `cargo test -- --ignored`).
    #[arg(long)]
    pub ignored: bool,

    /// Package to test.
    #[arg(short = 'p', long = "package")]
    pub packages: Vec<String>,

    /// Test target name.
    #[arg(long = "test")]
    pub tests: Vec<String>,

    /// Number of build jobs.
    #[arg(short = 'j', long)]
    pub jobs: Option<u32>,

    /// Features to enable.
    #[arg(short = 'F', long)]
    pub features: Vec<String>,

    /// Build in release mode.
    #[arg(long)]
    pub release: bool,

    /// Test only library.
    #[arg(long)]
    pub lib: bool,

    /// Don't stop on first failure.
    #[arg(long)]
    pub no_fail_fast: bool,

    /// Extra args passed after `--` to cargo/test binaries (filter, etc).
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

impl TestArgs {
    /// Build a `cargo test` command with all flags applied.
    /// Does NOT set stdout/stderr — caller decides piping.
    pub fn cargo_test_cmd(&self) -> Command {
        self.cargo_test_cmd_in(None)
    }

    /// Build a `cargo test` command, optionally running in a specific directory.
    pub fn cargo_test_cmd_in(&self, dir: Option<&Path>) -> Command {
        let mut cmd = Command::new("cargo");
        cmd.arg("test");
        cmd.env("RUSTFLAGS", crate::util::patchbay_rustflags());
        if let Some(d) = dir {
            cmd.current_dir(d);
        }
        for p in &self.packages {
            cmd.arg("-p").arg(p);
        }
        for t in &self.tests {
            cmd.arg("--test").arg(t);
        }
        if let Some(j) = self.jobs {
            cmd.arg("-j").arg(j.to_string());
        }
        for f in &self.features {
            cmd.arg("-F").arg(f);
        }
        if self.release {
            cmd.arg("--release");
        }
        if self.lib {
            cmd.arg("--lib");
        }
        if self.no_fail_fast {
            cmd.arg("--no-fail-fast");
        }
        // Everything after `--`: --ignored/--include-ignored + extra args
        if self.include_ignored || self.ignored || !self.extra_args.is_empty() {
            cmd.arg("--");
            if self.ignored {
                cmd.arg("--ignored");
            } else if self.include_ignored {
                cmd.arg("--include-ignored");
            }
            for a in &self.extra_args {
                cmd.arg(a);
            }
        }
        cmd
    }

    /// Convert to patchbay-vm TestVmArgs.
    #[cfg(feature = "vm")]
    pub fn into_vm_args(self, target: String, recreate: bool) -> patchbay_vm::TestVmArgs {
        let mut cargo_args = Vec::new();
        if let Some(j) = self.jobs {
            cargo_args.extend(["--jobs".into(), j.to_string()]);
        }
        for f in &self.features {
            cargo_args.extend(["--features".into(), f.clone()]);
        }
        if self.release {
            cargo_args.push("--release".into());
        }
        if self.lib {
            cargo_args.push("--lib".into());
        }
        if self.no_fail_fast {
            cargo_args.push("--no-fail-fast".into());
        }
        cargo_args.extend(self.extra_args);
        patchbay_vm::TestVmArgs {
            filter: None,
            target,
            packages: self.packages,
            tests: self.tests,
            recreate,
            cargo_args,
        }
    }
}

/// Resolve `target_directory` from cargo metadata.
fn cargo_target_dir() -> Option<PathBuf> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    meta["target_directory"].as_str().map(PathBuf::from)
}

/// Run tests natively via cargo test/nextest.
///
/// Captures stdout/stderr (printing live when `verbose` is true), parses
/// test results, and writes `run.json` to `testdir-current/`.
/// When `persist` is true, copies output to `.patchbay/work/run-{timestamp}/`.
pub fn run_native(args: TestArgs, verbose: bool, persist: bool) -> Result<()> {
    use std::io::BufRead;

    let use_nextest = has_nextest();
    let mut cmd = if use_nextest {
        let mut cmd = Command::new("cargo");
        cmd.arg("nextest").arg("run");
        cmd.env("RUSTFLAGS", crate::util::patchbay_rustflags());
        for p in &args.packages {
            cmd.arg("-p").arg(p);
        }
        for t in &args.tests {
            cmd.arg("--test").arg(t);
        }
        if let Some(j) = args.jobs {
            cmd.arg("-j").arg(j.to_string());
        }
        for f in &args.features {
            cmd.arg("-F").arg(f);
        }
        if args.release {
            cmd.arg("--release");
        }
        if args.lib {
            cmd.arg("--lib");
        }
        if args.no_fail_fast {
            cmd.arg("--no-fail-fast");
        }
        if args.include_ignored {
            cmd.arg("--run-ignored").arg("all");
        } else if args.ignored {
            cmd.arg("--run-ignored").arg("ignored-only");
        }
        for a in &args.extra_args {
            cmd.arg(a);
        }
        cmd
    } else {
        eprintln!("patchbay: cargo-nextest not found, using cargo test");
        args.cargo_test_cmd()
    };

    // Set PATCHBAY_OUTDIR so test fixtures can discover the output directory.
    if let Some(target_dir) = cargo_target_dir() {
        let outdir = target_dir.join("testdir-current");
        cmd.env("PATCHBAY_OUTDIR", &outdir);
    }

    // Pipe stdout/stderr so we can capture output while optionally printing live.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let started_at = chrono::Utc::now();
    let mut child = cmd.spawn().context("failed to spawn test command")?;

    let stdout_pipe = child.stdout.take().unwrap();
    let stderr_pipe = child.stderr.take().unwrap();
    let v = verbose;
    let out_t = std::thread::spawn(move || {
        let mut buf = String::new();
        for line in std::io::BufReader::new(stdout_pipe).lines().map_while(Result::ok) {
            if v { println!("{line}"); }
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });
    let err_t = std::thread::spawn(move || {
        let mut buf = String::new();
        for line in std::io::BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
            if verbose { eprintln!("{line}"); }
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let status = child.wait().context("failed to wait for test command")?;
    let ended_at = chrono::Utc::now();
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();

    let combined = format!("{stdout}\n{stderr}");
    let results = manifest::parse_test_output(&combined);

    // Write run.json into testdir-current/.
    let pass = results.iter().filter(|r| r.status == TestStatus::Pass).count() as u32;
    let fail = results.iter().filter(|r| r.status == TestStatus::Fail).count() as u32;
    let total = results.len() as u32;
    let git = manifest::git_context();
    let runtime = (ended_at - started_at).to_std().ok();
    let outcome = if status.success() { "pass" } else { "fail" };

    let manifest = RunManifest {
        kind: RunKind::Test,
        project: None,
        commit: git.commit,
        branch: git.branch,
        dirty: git.dirty,
        pr: None,
        pr_url: None,
        title: None,
        started_at: Some(started_at),
        ended_at: Some(ended_at),
        runtime,
        outcome: Some(outcome.to_string()),
        pass: Some(pass),
        fail: Some(fail),
        total: Some(total),
        tests: results,
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        patchbay_version: option_env!("CARGO_PKG_VERSION").map(|v| v.to_string()),
    };

    if let Some(target_dir) = cargo_target_dir() {
        let testdir = target_dir.join("testdir-current");
        std::fs::create_dir_all(&testdir).ok();
        let run_json = testdir.join("run.json");
        if let Ok(json) = serde_json::to_string_pretty(&manifest) {
            std::fs::write(&run_json, json).ok();
        }
    }

    // --persist: copy output dir to .patchbay/work/run-{timestamp}/
    if persist {
        persist_run()?;
    }

    if !status.success() {
        bail!("tests failed (exit code {})", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Copy testdir-current/ into `.patchbay/work/run-{timestamp}/`.
fn persist_run() -> Result<()> {
    let target_dir = cargo_target_dir().context("could not determine cargo target dir")?;
    let testdir = target_dir.join("testdir-current");
    if !testdir.exists() {
        return Ok(());
    }
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let dest = PathBuf::from(format!(".patchbay/work/run-{ts}"));
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let status = Command::new("cp")
        .args(["-r"])
        .arg(&testdir)
        .arg(&dest)
        .status()
        .context("cp testdir")?;
    if !status.success() {
        bail!("failed to copy testdir to {}", dest.display());
    }
    println!("patchbay: persisted run to {}", dest.display());
    Ok(())
}


/// Run tests in a VM via patchbay-vm.
#[cfg(feature = "vm")]
pub fn run_vm(args: TestArgs, backend: patchbay_vm::Backend) -> anyhow::Result<()> {
    use patchbay_vm::VmOps;
    let backend = backend.resolve();
    let target = patchbay_vm::default_test_target();
    backend.run_tests(args.into_vm_args(target, false))
}
