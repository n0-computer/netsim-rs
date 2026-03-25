//! Test command implementation.

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
    /// Test name filter.
    #[arg()]
    pub filter: Option<String>,

    /// Include ignored tests.
    #[arg(long)]
    pub ignored: bool,

    /// Run only ignored tests.
    #[arg(long)]
    pub ignored_only: bool,

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

    /// Extra args passed to cargo and test binaries.
    #[arg(last = true)]
    pub extra_args: Vec<String>,
}

impl TestArgs {
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
            filter: self.filter,
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
    if !use_nextest {
        eprintln!("patchbay: cargo-nextest not found, using cargo test (nextest recommended for structured output)");
    }

    let mut cmd = Command::new("cargo");
    if use_nextest {
        cmd.arg("nextest").arg("run");
    } else {
        cmd.arg("test");
    }

    // Add RUSTFLAGS with cfg(patchbay_tests)
    cmd.env("RUSTFLAGS", crate::util::patchbay_rustflags());

    // Package selectors
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

    // Extra cargo args
    for a in &args.extra_args {
        cmd.arg(a);
    }

    // For cargo test (not nextest), filter and --ignored go after --
    if use_nextest {
        if let Some(ref f) = args.filter {
            cmd.arg("-E").arg(format!("test(/{f}/)"));
        }
        if args.ignored {
            cmd.arg("--run-ignored").arg("all");
        } else if args.ignored_only {
            cmd.arg("--run-ignored").arg("ignored-only");
        }
    } else {
        // cargo test: filter before --, ignored flags after --
        if let Some(ref f) = args.filter {
            cmd.arg(f);
        }
        if args.ignored || args.ignored_only {
            cmd.arg("--");
            if args.ignored_only {
                cmd.arg("--ignored");
            } else {
                cmd.arg("--include-ignored");
            }
        }
    }

    let status = cmd.status().context("failed to run cargo test")?;
    if !status.success() {
        bail!("tests failed (exit code {})", status.code().unwrap_or(-1));
    }
    copy_testdir_output();
    Ok(())
}

/// Copy testdir-current into the work dir if it exists.
fn copy_testdir_output() {
    // Try to find target/testdir-current via cargo metadata
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
    // Use cp -r since std::fs doesn't have recursive copy
    let _ = Command::new("cp")
        .args(["-r"])
        .arg(&testdir)
        .arg(dest)
        .status();
}

/// Run tests in a VM via patchbay-vm.
#[cfg(feature = "vm")]
pub fn run_vm(args: TestArgs, backend: patchbay_vm::Backend) -> anyhow::Result<()> {
    use patchbay_vm::VmOps;
    let backend = backend.resolve();
    let target = patchbay_vm::default_test_target();
    backend.run_tests(args.into_vm_args(target, false))
}
