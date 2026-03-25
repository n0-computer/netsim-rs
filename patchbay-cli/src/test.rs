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

pub struct TestArgs {
    pub filter: Option<String>,
    pub ignored: bool,
    pub ignored_only: bool,
    pub packages: Vec<String>,
    pub tests: Vec<String>,
    pub jobs: Option<u32>,
    pub features: Vec<String>,
    pub release: bool,
    pub lib: bool,
    pub no_fail_fast: bool,
    pub extra_args: Vec<String>,
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

    // Add RUSTFLAGS with cfg(patchbay_test)
    let existing_rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let rustflags = if existing_rustflags.is_empty() {
        "--cfg patchbay_test".to_string()
    } else {
        format!("{existing_rustflags} --cfg patchbay_test")
    };
    cmd.env("RUSTFLAGS", &rustflags);

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
    Ok(())
}
