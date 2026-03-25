//! Compare mode: run tests/sims in two git worktrees and diff results.

use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Set up a git worktree for the given ref.
pub fn setup_worktree(git_ref: &str, base: &Path) -> Result<PathBuf> {
    let tree_dir = base.join(".patchbay/tree").join(sanitize_ref(git_ref));
    if tree_dir.exists() {
        // Remove existing worktree first
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&tree_dir)
            .status();
    }
    std::fs::create_dir_all(tree_dir.parent().unwrap())?;
    let status = Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&tree_dir)
        .arg(git_ref)
        .status()
        .context("git worktree add")?;
    if !status.success() {
        bail!("failed to create worktree for ref '{git_ref}'");
    }
    Ok(tree_dir)
}

/// Remove worktree if it has no changes.
pub fn cleanup_worktree(tree_dir: &Path) -> Result<()> {
    let diff = Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(tree_dir)
        .status()
        .context("git diff")?;
    if diff.success() {
        // No changes, safe to remove
        let _ = Command::new("git")
            .args(["worktree", "remove"])
            .arg(tree_dir)
            .status();
    }
    Ok(())
}

fn sanitize_ref(r: &str) -> String {
    r.replace('/', "_").replace('\\', "_")
}

// ── Test comparison ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Pass,
    Fail,
    Ignored,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompareManifest {
    pub left_ref: String,
    pub right_ref: String,
    pub timestamp: String,
    /// Project name (for CI upload scoping).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub left_results: Vec<TestResult>,
    pub right_results: Vec<TestResult>,
    pub summary: CompareSummary,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompareSummary {
    pub left_pass: usize,
    pub left_fail: usize,
    pub left_total: usize,
    pub right_pass: usize,
    pub right_fail: usize,
    pub right_total: usize,
    pub fixes: usize,
    pub regressions: usize,
    pub left_time_ms: u64,
    pub right_time_ms: u64,
    pub score: i32,
}

/// Parse cargo test output into TestResults.
/// Parses lines like "test tests::foo ... ok" and "test tests::bar ... FAILED".
pub fn parse_test_output(output: &str) -> Vec<TestResult> {
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("test ") {
            continue;
        }
        // "test path::to::test ... ok"
        // "test path::to::test ... FAILED"
        // "test path::to::test ... ignored"
        if let Some(rest) = line.strip_prefix("test ") {
            if let Some((name, outcome)) = rest.rsplit_once(" ... ") {
                let name = name.trim().to_string();
                let status = match outcome.trim() {
                    "ok" => TestStatus::Pass,
                    "FAILED" => TestStatus::Fail,
                    "ignored" => TestStatus::Ignored,
                    _ => continue,
                };
                results.push(TestResult { name, status, duration_ms: None });
            }
        }
    }
    results
}

/// Run tests in a directory and capture results.
pub fn run_tests_in_dir(
    dir: &Path,
    filter: &Option<String>,
    ignored: bool,
    ignored_only: bool,
    packages: &[String],
    tests: &[String],
) -> Result<(Vec<TestResult>, String)> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(dir);
    cmd.arg("test");

    // Add RUSTFLAGS
    let existing = std::env::var("RUSTFLAGS").unwrap_or_default();
    let rustflags = if existing.is_empty() {
        "--cfg patchbay_test".to_string()
    } else {
        format!("{existing} --cfg patchbay_test")
    };
    cmd.env("RUSTFLAGS", &rustflags);

    for p in packages {
        cmd.arg("-p").arg(p);
    }
    for t in tests {
        cmd.arg("--test").arg(t);
    }

    if let Some(f) = filter {
        cmd.arg(f);
    }
    if ignored || ignored_only {
        cmd.arg("--");
        if ignored_only {
            cmd.arg("--ignored");
        } else {
            cmd.arg("--include-ignored");
        }
    }

    let output = cmd.output().context("run cargo test")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}\n{stderr}");
    let results = parse_test_output(&combined);
    Ok((results, combined))
}

/// Compare two sets of test results.
pub fn compare_results(
    _left_ref: &str,
    _right_ref: &str,
    left: &[TestResult],
    right: &[TestResult],
) -> CompareSummary {
    use std::collections::HashMap;
    let left_map: HashMap<&str, &TestResult> = left.iter().map(|r| (r.name.as_str(), r)).collect();
    let right_map: HashMap<&str, &TestResult> = right.iter().map(|r| (r.name.as_str(), r)).collect();

    let left_pass = left.iter().filter(|r| r.status == TestStatus::Pass).count();
    let left_fail = left.iter().filter(|r| r.status == TestStatus::Fail).count();
    let right_pass = right.iter().filter(|r| r.status == TestStatus::Pass).count();
    let right_fail = right.iter().filter(|r| r.status == TestStatus::Fail).count();

    let mut fixes = 0;
    let mut regressions = 0;
    // All test names from both sides
    let mut all_names: Vec<&str> = left_map.keys().chain(right_map.keys()).copied().collect();
    all_names.sort();
    all_names.dedup();
    for name in &all_names {
        let ls = left_map.get(name).map(|r| r.status);
        let rs = right_map.get(name).map(|r| r.status);
        match (ls, rs) {
            (Some(TestStatus::Fail), Some(TestStatus::Pass)) => fixes += 1,
            (Some(TestStatus::Pass), Some(TestStatus::Fail)) => regressions += 1,
            _ => {}
        }
    }

    let left_time_ms: u64 = left.iter().filter_map(|r| r.duration_ms).sum();
    let right_time_ms: u64 = right.iter().filter_map(|r| r.duration_ms).sum();

    // Scoring
    let mut score: i32 = 0;
    score += fixes as i32 * 3;
    score -= regressions as i32 * 5;
    if left_time_ms > 0 {
        let time_pct = (right_time_ms as f64 - left_time_ms as f64) / left_time_ms as f64 * 100.0;
        if time_pct < -2.0 { score += 1; }
        if time_pct > 5.0 { score -= 1; }
    }

    CompareSummary {
        left_pass, left_fail, left_total: left.len(),
        right_pass, right_fail, right_total: right.len(),
        fixes, regressions,
        left_time_ms, right_time_ms,
        score,
    }
}

/// Print a comparison summary table.
pub fn print_summary(left_ref: &str, right_ref: &str, left: &[TestResult], right: &[TestResult], summary: &CompareSummary) {
    use std::collections::HashMap;
    println!("\nCompare: {left_ref} \u{2194} {right_ref}\n");
    println!("Tests:        {}/{} pass \u{2192} {}/{} pass",
        summary.left_pass, summary.left_total,
        summary.right_pass, summary.right_total);
    if summary.fixes > 0 {
        println!("Fixes:        {} (fail\u{2192}pass)", summary.fixes);
    }
    if summary.regressions > 0 {
        println!("Regressions:  {} (pass\u{2192}fail)", summary.regressions);
    }
    if summary.left_time_ms > 0 || summary.right_time_ms > 0 {
        println!("Total time:   {:.1}s \u{2192} {:.1}s",
            summary.left_time_ms as f64 / 1000.0,
            summary.right_time_ms as f64 / 1000.0);
    }

    // Per-test table
    let left_map: HashMap<&str, &TestResult> = left.iter().map(|r| (r.name.as_str(), r)).collect();
    let right_map: HashMap<&str, &TestResult> = right.iter().map(|r| (r.name.as_str(), r)).collect();
    let mut all_names: Vec<&str> = left_map.keys().chain(right_map.keys()).copied().collect();
    all_names.sort();
    all_names.dedup();

    println!("\n{:<50} {:>8} {:>8} {:>10}", "Test", "Left", "Right", "Delta");
    println!("{}", "-".repeat(80));
    for name in &all_names {
        let ls = left_map.get(name).map(|r| r.status);
        let rs = right_map.get(name).map(|r| r.status);
        let ls_str = match ls {
            Some(TestStatus::Pass) => "PASS",
            Some(TestStatus::Fail) => "FAIL",
            Some(TestStatus::Ignored) => "SKIP",
            None => "-",
        };
        let rs_str = match rs {
            Some(TestStatus::Pass) => "PASS",
            Some(TestStatus::Fail) => "FAIL",
            Some(TestStatus::Ignored) => "SKIP",
            None => "-",
        };
        let delta = match (ls, rs) {
            (Some(TestStatus::Fail), Some(TestStatus::Pass)) => "fixed",
            (Some(TestStatus::Pass), Some(TestStatus::Fail)) => "REGRESS",
            (None, Some(_)) => "new",
            (Some(_), None) => "removed",
            _ => "",
        };
        // Truncate long test names
        let display_name = if name.len() > 48 { &name[name.len()-48..] } else { name };
        println!("{:<50} {:>8} {:>8} {:>10}", display_name, ls_str, rs_str, delta);
    }

    println!("\nScore: {:+} ({} fixes, {} regressions)", summary.score, summary.fixes, summary.regressions);
}

/// Upload a directory to a patchbay-server instance via tar.gz push.
///
/// Uses the existing `POST /api/push/{project}` endpoint. Shells out to
/// `tar` and `curl` to keep dependencies minimal.
pub fn upload(dir: &Path, project: &str, url: &str, api_key: &str) -> Result<()> {
    // tar cz the directory contents
    let tar = Command::new("tar")
        .args(["czf", "-", "-C"])
        .arg(dir)
        .arg(".")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn tar")?;

    let push_url = format!("{}/api/push/{}", url.trim_end_matches('/'), project);
    let status = Command::new("curl")
        .args(["-sf", "--data-binary", "@-"])
        .arg("-H")
        .arg(format!("Authorization: Bearer {api_key}"))
        .arg("-H")
        .arg("Content-Type: application/gzip")
        .arg(&push_url)
        .stdin(tar.stdout.unwrap())
        .status()
        .context("failed to run curl")?;
    if !status.success() {
        bail!("upload failed (curl exit {})", status.code().unwrap_or(-1));
    }
    println!("uploaded to {push_url}");
    Ok(())
}
