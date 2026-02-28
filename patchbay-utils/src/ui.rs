//! UI build and server helpers.

use std::{
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};

pub use crate::serve::{start_ui_server, DEFAULT_UI_BIND};

/// Rebuild the embedded UI from source.
pub fn build_ui(ui_dir: &Path) -> Result<()> {
    run_npm(ui_dir, &["install"])?;
    run_npm(ui_dir, &["run", "build"])
}

fn run_npm(ui_dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(ui_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to run npm {}", args.join(" ")))?;
    if !status.success() {
        anyhow::bail!("npm {} failed with status {status}", args.join(" "));
    }
    Ok(())
}
