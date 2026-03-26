//! Compare mode: run tests/sims in two git worktrees and diff results.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
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

/// Remove worktree if tracked files are unchanged.
/// Uses --force to handle untracked files (e.g. target/).
pub fn cleanup_worktree(tree_dir: &Path) -> Result<()> {
    let diff = Command::new("git")
        .args(["diff", "--quiet"])
        .current_dir(tree_dir)
        .status()
        .context("git diff")?;
    if diff.success() {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(tree_dir)
            .status();
    }
    Ok(())
}

fn sanitize_ref(r: &str) -> String {
    r.replace(['/', '\\'], "_")
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStats {
    pub pass: usize,
    pub fail: usize,
    pub total: usize,
    #[serde(with = "duration_ms")]
    pub time: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompareSummary {
    pub left: RunStats,
    pub right: RunStats,
    pub fixes: usize,
    pub regressions: usize,
    pub score: i32,
}

/// Serialize Duration as milliseconds.
mod duration_ms {
    use std::time::Duration;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Parse cargo test output into TestResults.
/// Parses lines like "test tests::foo ... ok" and "test tests::bar ... FAILED".
pub fn parse_test_output(output: &str) -> Vec<TestResult> {
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();
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
    args: &crate::test::TestArgs,
    verbose: bool,
) -> Result<(Vec<TestResult>, String)> {
    use std::io::BufRead;

    let mut cmd = args.cargo_test_cmd_in(Some(dir));
    // Use a per-worktree target dir to avoid sharing cached binaries
    // between different git refs.
    cmd.env("CARGO_TARGET_DIR", dir.join("target"));
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().context("spawn cargo test")?;

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

    let _ = child.wait().context("wait for cargo test")?;
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    let combined = format!("{stdout}\n{stderr}");
    let results = parse_test_output(&combined);
    Ok((results, combined))
}

fn test_index(results: &[TestResult]) -> std::collections::HashMap<&str, &TestResult> {
    results.iter().map(|r| (r.name.as_str(), r)).collect()
}

fn merged_names(left: &[TestResult], right: &[TestResult]) -> Vec<String> {
    let mut names: Vec<String> = left.iter().chain(right.iter()).map(|r| r.name.clone()).collect();
    names.sort();
    names.dedup();
    names
}

/// Compare two sets of test results.
pub fn compare_results(
    _left_ref: &str,
    _right_ref: &str,
    left: &[TestResult],
    right: &[TestResult],
) -> CompareSummary {
    let left_map = test_index(left);
    let right_map = test_index(right);

    let left_pass = left.iter().filter(|r| r.status == TestStatus::Pass).count();
    let left_fail = left.iter().filter(|r| r.status == TestStatus::Fail).count();
    let right_pass = right.iter().filter(|r| r.status == TestStatus::Pass).count();
    let right_fail = right.iter().filter(|r| r.status == TestStatus::Fail).count();

    let mut fixes = 0;
    let mut regressions = 0;
    let all_names = merged_names(left, right);
    for name in &all_names {
        let name = name.as_str();
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
        left: RunStats {
            pass: left_pass,
            fail: left_fail,
            total: left.len(),
            time: Duration::from_millis(left_time_ms),
        },
        right: RunStats {
            pass: right_pass,
            fail: right_fail,
            total: right.len(),
            time: Duration::from_millis(right_time_ms),
        },
        fixes, regressions,
        score,
    }
}

/// Print a comparison summary table.
pub fn print_summary(left_ref: &str, right_ref: &str, left: &[TestResult], right: &[TestResult], summary: &CompareSummary) {
    println!("\nCompare: {left_ref} \u{2194} {right_ref}\n");
    println!("Tests:        {}/{} pass \u{2192} {}/{} pass",
        summary.left.pass, summary.left.total,
        summary.right.pass, summary.right.total);
    if summary.fixes > 0 {
        println!("Fixes:        {} (fail\u{2192}pass)", summary.fixes);
    }
    if summary.regressions > 0 {
        println!("Regressions:  {} (pass\u{2192}fail)", summary.regressions);
    }
    if !summary.left.time.is_zero() || !summary.right.time.is_zero() {
        println!("Total time:   {:.1}s \u{2192} {:.1}s",
            summary.left.time.as_secs_f64(),
            summary.right.time.as_secs_f64());
    }

    // Per-test table
    let left_map = test_index(left);
    let right_map = test_index(right);
    let all_names = merged_names(left, right);

    println!("\n{:<50} {:>8} {:>8} {:>10}", "Test", "Left", "Right", "Delta");
    println!("{}", "-".repeat(80));
    for name in &all_names {
        let name = name.as_str();
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
