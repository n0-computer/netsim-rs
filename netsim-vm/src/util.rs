use anyhow::{bail, Context, Result};
use netsim_utils::assets::{
    parse_binary_overrides, resolve_binary_source_path, BinaryOverride, PathResolveMode,
};
use netsim_utils::binary_cache::{cached_binary_for_url, set_executable};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve `--binary` override arguments, copy the resulting binaries into
/// `<work_dir>/binaries/`, and return rewritten `"name:path:/work/binaries/..."` overrides
/// for forwarding to the in-VM netsim invocation.
pub fn stage_binary_overrides(
    raw: &[String],
    work_dir: &Path,
    target_dir: &Path,
    target: &str,
) -> Result<Vec<String>> {
    let parsed = parse_binary_overrides(raw)?;
    let bins_dir = work_dir.join("binaries");
    std::fs::create_dir_all(&bins_dir).with_context(|| format!("create {}", bins_dir.display()))?;

    let mut rewritten = Vec::new();
    for (name, ov) in parsed {
        let staged = match ov {
            BinaryOverride::Path(src) => stage_path_binary(&name, &src, &bins_dir)?,
            BinaryOverride::Fetch(url) => stage_fetch_binary(&name, &url, work_dir, &bins_dir)?,
            BinaryOverride::Build(src) => {
                stage_build_binary(&name, &src, &bins_dir, target_dir, target)?
            }
        };
        let guest = format!(
            "/work/binaries/{}",
            staged.file_name().and_then(|s| s.to_str()).unwrap_or("bin")
        );
        rewritten.push(format!("{name}:path:{guest}"));
    }
    Ok(rewritten)
}

fn stage_path_binary(name: &str, src: &Path, bins_dir: &Path) -> Result<PathBuf> {
    let resolved = resolve_binary_source_path(src, PathResolveMode::Vm)?;
    if !resolved.exists() || resolved.is_dir() {
        bail!(
            "binary override path for '{}' is invalid: {}",
            name,
            resolved.display()
        );
    }
    let dest = bins_dir.join(format!("{}-override", name));
    std::fs::copy(&resolved, &dest)
        .with_context(|| format!("copy {} -> {}", resolved.display(), dest.display()))?;
    set_executable(&dest)?;
    Ok(dest)
}

fn stage_fetch_binary(name: &str, url: &str, work_dir: &Path, bins_dir: &Path) -> Result<PathBuf> {
    let cached = cached_binary_for_url(url, &work_dir.join(".binary-cache"))?;
    let file_name = cached
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("binary");
    let dest = bins_dir.join(format!("{}-fetch-{}", name, file_name));
    std::fs::copy(&cached, &dest)
        .with_context(|| format!("copy {} -> {}", cached.display(), dest.display()))?;
    set_executable(&dest)?;
    Ok(dest)
}

fn stage_build_binary(
    name: &str,
    src: &Path,
    bins_dir: &Path,
    target_dir: &Path,
    target: &str,
) -> Result<PathBuf> {
    if !src.is_dir() {
        bail!(
            "build source for '{}' is not a directory: {}",
            name,
            src.display()
        );
    }

    let example = Command::new("cargo")
        .args(["build", "--release", "--target", target, "--example", name])
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(src)
        .status()
        .context("spawn cargo build --example")?;

    let built = if example.success() {
        target_dir.join(target).join("release").join(name)
    } else {
        let bin = Command::new("cargo")
            .args(["build", "--release", "--target", target, "--bin", name])
            .env("CARGO_TARGET_DIR", target_dir)
            .current_dir(src)
            .status()
            .context("spawn cargo build --bin")?;
        if !bin.success() {
            bail!(
                "failed to build '{}' as example or bin in {}",
                name,
                src.display()
            );
        }
        target_dir.join(target).join("release").join(name)
    };

    if !built.exists() {
        bail!("expected built binary not found: {}", built.display());
    }
    let dest = bins_dir.join(format!("{}-build", name));
    std::fs::copy(&built, &dest)
        .with_context(|| format!("copy {} -> {}", built.display(), dest.display()))?;
    set_executable(&dest)?;
    Ok(dest)
}
