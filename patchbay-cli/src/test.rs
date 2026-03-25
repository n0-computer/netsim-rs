//! Test command implementation.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

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

/// Run tests natively via cargo test/nextest.
pub fn run_native(args: TestArgs) -> Result<()> {
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
        // nextest: extra_args go directly (filter is just a positional)
        for a in &args.extra_args {
            cmd.arg(a);
        }
        cmd
    } else {
        eprintln!("patchbay: cargo-nextest not found, using cargo test");
        args.cargo_test_cmd()
    };

    let status = cmd.status().context("failed to run tests")?;
    if !status.success() {
        bail!("tests failed (exit code {})", status.code().unwrap_or(-1));
    }
    copy_testdir_output();
    Ok(())
}

/// Copy testdir-current into the work dir if it exists.
fn copy_testdir_output() {
    let Ok(output) = Command::new("cargo")
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return;
    };
    let Some(target_dir) = meta["target_directory"].as_str() else {
        return;
    };
    let testdir = std::path::Path::new(target_dir).join("testdir-current");
    if !testdir.exists() {
        return;
    }
    let dest = std::path::Path::new(".patchbay/work/testdir");
    if dest.exists() {
        let _ = std::fs::remove_dir_all(dest);
    }
    let _ = Command::new("cp").args(["-r"]).arg(&testdir).arg(dest).status();
}

/// Run tests in a VM via patchbay-vm.
#[cfg(feature = "vm")]
pub fn run_vm(args: TestArgs, backend: patchbay_vm::Backend) -> anyhow::Result<()> {
    use patchbay_vm::VmOps;
    let backend = backend.resolve();
    let target = patchbay_vm::default_test_target();
    backend.run_tests(args.into_vm_args(target, false))
}
