//! Compare mode: run tests/sims in two git worktrees and diff results.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use anyhow::{bail, Context, Result};
use patchbay_utils::manifest::{self, TestResult, TestStatus};

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

// Types re-exported from patchbay_utils::manifest:
// TestResult, TestStatus, RunManifest, RunKind

pub use manifest::parse_test_output;

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

/// Persist test results from a worktree run so future compares can reuse them.
///
/// Writes `run.json` into `.patchbay/work/run-{timestamp}/`.
pub fn persist_worktree_run(
    _tree_dir: &Path,
    results: &[TestResult],
    commit_sha: &str,
) -> Result<()> {
    use manifest::{RunKind, RunManifest};

    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let dest = PathBuf::from(format!(".patchbay/work/run-{ts}"));
    std::fs::create_dir_all(&dest)?;

    let pass = results.iter().filter(|r| r.status == TestStatus::Pass).count() as u32;
    let fail = results.iter().filter(|r| r.status == TestStatus::Fail).count() as u32;
    let total = results.len() as u32;
    let outcome = if fail == 0 { "pass" } else { "fail" };

    let manifest = RunManifest {
        kind: RunKind::Test,
        project: None,
        commit: Some(commit_sha.to_string()),
        branch: None,
        dirty: false,
        pr: None,
        pr_url: None,
        title: None,
        started_at: None,
        ended_at: None,
        runtime: None,
        outcome: Some(outcome.to_string()),
        pass: Some(pass),
        fail: Some(fail),
        total: Some(total),
        tests: results.to_vec(),
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        patchbay_version: option_env!("CARGO_PKG_VERSION").map(|v| v.to_string()),
    };

    let json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(dest.join("run.json"), json)?;
    println!("patchbay: persisted run to {}", dest.display());
    Ok(())
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

/// Aggregate pass/fail/total for one side of a comparison.
pub struct SideStats {
    pub pass: usize,
    pub fail: usize,
    pub total: usize,
}

/// Computed comparison result (not persisted — compare is always computed on the fly).
pub struct CompareResult {
    pub left: SideStats,
    pub right: SideStats,
    pub fixes: usize,
    pub regressions: usize,
    pub score: i32,
}

/// Compare two sets of test results and return computed stats.
pub fn compare_results(left: &[TestResult], right: &[TestResult]) -> CompareResult {
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
        let ls = left_map.get(name.as_str()).map(|r| r.status);
        let rs = right_map.get(name.as_str()).map(|r| r.status);
        match (ls, rs) {
            (Some(TestStatus::Fail), Some(TestStatus::Pass)) => fixes += 1,
            (Some(TestStatus::Pass), Some(TestStatus::Fail)) => regressions += 1,
            _ => {}
        }
    }

    let left_time: Duration = left.iter().filter_map(|r| r.duration).sum();
    let right_time: Duration = right.iter().filter_map(|r| r.duration).sum();

    let mut score: i32 = 0;
    score += fixes as i32 * 3;
    score -= regressions as i32 * 5;
    if !left_time.is_zero() {
        let pct = (right_time.as_secs_f64() - left_time.as_secs_f64()) / left_time.as_secs_f64() * 100.0;
        if pct < -2.0 { score += 1; }
        if pct > 5.0 { score -= 1; }
    }

    CompareResult {
        left: SideStats { pass: left_pass, fail: left_fail, total: left.len() },
        right: SideStats { pass: right_pass, fail: right_fail, total: right.len() },
        fixes, regressions, score,
    }
}

fn status_str(s: TestStatus) -> &'static str {
    match s {
        TestStatus::Pass => "PASS",
        TestStatus::Fail => "FAIL",
        TestStatus::Ignored => "SKIP",
    }
}

/// Print a comparison summary table.
pub fn print_summary(left_ref: &str, right_ref: &str, left: &[TestResult], right: &[TestResult], result: &CompareResult) {
    println!("\nCompare: {left_ref} \u{2194} {right_ref}\n");
    println!("Tests:        {}/{} pass ({} fail) \u{2192} {}/{} pass ({} fail)",
        result.left.pass, result.left.total, result.left.fail,
        result.right.pass, result.right.total, result.right.fail);
    if result.fixes > 0 {
        println!("Fixes:        {} (fail\u{2192}pass)", result.fixes);
    }
    if result.regressions > 0 {
        println!("Regressions:  {} (pass\u{2192}fail)", result.regressions);
    }

    let left_map = test_index(left);
    let right_map = test_index(right);
    let all_names = merged_names(left, right);

    println!("\n{:<50} {:>8} {:>8} {:>10}", "Test", "Left", "Right", "Delta");
    println!("{}", "-".repeat(80));
    for name in &all_names {
        let name = name.as_str();
        let ls = left_map.get(name).map(|r| r.status);
        let rs = right_map.get(name).map(|r| r.status);
        let ls_str = ls.map(status_str).unwrap_or("-");
        let rs_str = rs.map(status_str).unwrap_or("-");
        let delta = match (ls, rs) {
            (Some(TestStatus::Fail), Some(TestStatus::Pass)) => "fixed",
            (Some(TestStatus::Pass), Some(TestStatus::Fail)) => "REGRESS",
            (None, Some(_)) => "new",
            (Some(_), None) => "removed",
            _ => "",
        };
        let display_name = if name.len() > 48 { &name[name.len()-48..] } else { name };
        println!("{:<50} {:>8} {:>8} {:>10}", display_name, ls_str, rs_str, delta);
    }

    println!("\nScore: {:+} ({} fixes, {} regressions)", result.score, result.fixes, result.regressions);
}
