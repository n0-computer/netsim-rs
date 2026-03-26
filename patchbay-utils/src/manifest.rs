//! Unified run manifest types shared across the patchbay workspace.
//!
//! Every execution (test or sim) writes a `run.json` manifest with git context.
//! This module defines the canonical types for that manifest.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Duration serde helpers ──────────────────────────────────────────

/// Serialize/deserialize a [`Duration`] as integer milliseconds.
pub mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

/// Serialize/deserialize an `Option<Duration>` as integer milliseconds.
pub mod option_duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_u64(d.as_millis() as u64),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?.map(Duration::from_millis))
    }
}

// ── Core types ──────────────────────────────────────────────────────

/// What produced a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunKind {
    Test,
    #[default]
    Sim,
}

/// Per-test pass/fail/ignored status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Pass,
    Fail,
    Ignored,
}

/// A single test result with name, status, and optional duration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    /// Test duration, serialized as integer milliseconds.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_duration_ms"
    )]
    pub duration: Option<Duration>,
    /// Relative directory path for this test's output (e.g. `"patchbay/holepunch_simple"`).
    /// Populated by the server when the directory exists on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// Unified manifest written as `run.json` alongside every run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    // ── Identity ──
    #[serde(default)]
    pub kind: RunKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    // ── Git context ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub dirty: bool,

    // ── CI context (populated from env vars when available) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    // ── Execution ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// Total runtime, serialized as integer milliseconds.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_duration_ms"
    )]
    pub runtime: Option<Duration>,

    // ── Outcome ──
    /// "pass" or "fail". Aliases for backward compat with old run.json fields.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "test_outcome",
        alias = "status"
    )]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,

    // ── Per-test results (kind == Test only) ──
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<TestResult>,

    // ── Environment ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patchbay_version: Option<String>,
}

impl RunManifest {
    /// Populate `dir` fields by scanning the run directory for subdirs that
    /// contain `events.jsonl`, then matching them to test results by the bare
    /// function name (last path segment of the dir, last token of the nextest name).
    pub fn resolve_test_dirs(&mut self, run_dir: &std::path::Path) {
        // Collect all dirs with events.jsonl, recursively (up to 2 levels).
        let mut test_dirs: Vec<String> = Vec::new();
        collect_event_dirs(run_dir, run_dir, 0, 2, &mut test_dirs);

        // Build a map: bare function name → relative dir path.
        // e.g. "holepunch_simple" → "patchbay/holepunch_simple"
        let dir_by_fn: std::collections::HashMap<&str, &str> = test_dirs
            .iter()
            .filter_map(|d| {
                let fn_name = d.rsplit('/').next()?;
                Some((fn_name, d.as_str()))
            })
            .collect();

        // Match each test result to a directory by bare function name.
        // Nextest name: "iroh::patchbay holepunch_simple" → last token "holepunch_simple"
        for test in &mut self.tests {
            let fn_name = test
                .name
                .rsplit_once(' ')
                .map(|(_, name)| name)
                .unwrap_or(&test.name);
            if let Some(&dir) = dir_by_fn.get(fn_name) {
                test.dir = Some(dir.to_string());
            }
        }
    }
}

/// Recursively collect relative paths to directories containing `events.jsonl`.
fn collect_event_dirs(
    root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("events.jsonl").exists() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        } else {
            collect_event_dirs(root, &path, depth + 1, max_depth, out);
        }
    }
}

// ── Git helpers ─────────────────────────────────────────────────────

/// Snapshot of git repository state.
pub struct GitContext {
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub dirty: bool,
}

/// Capture the current git HEAD commit, branch, and dirty state.
pub fn git_context() -> GitContext {
    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());
    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| s != "HEAD");
    // Check both unstaged and staged changes.
    let unstaged = !Command::new("git")
        .args(["diff", "--quiet"])
        .status()
        .map(|s| s.success())
        .unwrap_or(true);
    let staged = !Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .status()
        .map(|s| s.success())
        .unwrap_or(true);
    let dirty = unstaged || staged;
    GitContext {
        commit,
        branch,
        dirty,
    }
}

/// Resolve a git ref (branch name, tag, or SHA prefix) to a full commit SHA.
pub fn resolve_ref(git_ref: &str) -> Option<String> {
    Command::new("git")
        .args(["rev-parse", git_ref])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

// ── Run lookup ──────────────────────────────────────────────────────

/// Find a persisted run matching commit SHA and kind.
///
/// Scans `work_dir/*/run.json` for a manifest whose `commit` and `kind`
/// match and whose `dirty` flag is `false`.
pub fn find_run_for_commit(
    work_dir: &Path,
    commit: &str,
    kind: RunKind,
) -> Option<(PathBuf, RunManifest)> {
    for entry in fs::read_dir(work_dir).ok()?.flatten() {
        let run_json = entry.path().join("run.json");
        if let Ok(text) = fs::read_to_string(&run_json) {
            if let Ok(m) = serde_json::from_str::<RunManifest>(&text) {
                if m.kind == kind && m.commit.as_deref() == Some(commit) && !m.dirty {
                    return Some((entry.path(), m));
                }
            }
        }
    }
    None
}

// ── Test output parsing ─────────────────────────────────────────────

/// Parse `cargo test` and `cargo nextest` stdout into per-test results.
///
/// Recognises two formats:
/// - cargo test:  `test some::path ... ok`
/// - nextest:     `    PASS [   1.234s] crate::module::test_name`
pub fn parse_test_output(output: &str) -> Vec<TestResult> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();

        // cargo test format: "test name ... ok|FAILED|ignored"
        if let Some(rest) = line.strip_prefix("test ") {
            if let Some((name, status_str)) = rest.rsplit_once(" ... ") {
                let status = match status_str.trim() {
                    "ok" => TestStatus::Pass,
                    "FAILED" => TestStatus::Fail,
                    "ignored" => TestStatus::Ignored,
                    _ => continue,
                };
                let name = name.trim().to_string();
                if seen.insert(name.clone()) {
                    results.push(TestResult {
                        name,
                        status,
                        duration: None,
                        dir: None,
                    });
                }
            }
            continue;
        }

        // nextest format: "PASS [   1.234s] crate::test_name"
        //                 "FAIL [   0.567s] crate::test_name"
        //                 "IGNORE           crate::test_name"
        //                 "TIMEOUT [ 60.0s] crate::test_name"
        if let Some((status, rest)) = parse_nextest_line(line) {
            let duration = parse_nextest_duration(rest);
            let name = rest
                .find(']')
                .map(|i| &rest[i + 1..])
                .unwrap_or(rest)
                .trim()
                .to_string();
            if !name.is_empty() && seen.insert(name.clone()) {
                results.push(TestResult {
                    name,
                    status,
                    duration,
                    dir: None,
                });
            }
        }
    }
    results
}

fn parse_nextest_line(line: &str) -> Option<(TestStatus, &str)> {
    let prefixes = [
        ("PASS", TestStatus::Pass),
        ("FAIL", TestStatus::Fail),
        ("IGNORE", TestStatus::Ignored),
        ("TIMEOUT", TestStatus::Fail),
    ];
    for (prefix, status) in prefixes {
        if let Some(rest) = line.strip_prefix(prefix) {
            if rest.starts_with(' ') || rest.starts_with('[') {
                return Some((status, rest.trim()));
            }
        }
    }
    None
}

fn parse_nextest_duration(s: &str) -> Option<Duration> {
    // "[   1.234s] name" → extract "1.234"
    let s = s.strip_prefix('[')?;
    let end = s.find(']')?;
    let inner = s[..end].trim().strip_suffix('s')?;
    let secs: f64 = inner.parse().ok()?;
    Some(Duration::from_secs_f64(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_test_output() {
        let output = "\
running 3 tests
test foo::bar ... ok
test baz ... FAILED
test qux ... ignored

test result: FAILED. 1 passed; 1 failed; 1 ignored;
";
        let results = parse_test_output(output);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "foo::bar");
        assert_eq!(results[0].status, TestStatus::Pass);
        assert_eq!(results[1].name, "baz");
        assert_eq!(results[1].status, TestStatus::Fail);
        assert_eq!(results[2].name, "qux");
        assert_eq!(results[2].status, TestStatus::Ignored);
    }

    #[test]
    fn test_parse_nextest_output() {
        let output = "\
    Compiling my-crate v0.1.0
        PASS [   1.234s] my-crate::tests::foo
        FAIL [   0.567s] my-crate::tests::bar
     TIMEOUT [  60.001s] my-crate::tests::baz
      IGNORE            my-crate::tests::qux
";
        let results = parse_test_output(output);
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].name, "my-crate::tests::foo");
        assert_eq!(results[0].status, TestStatus::Pass);
        assert_eq!(results[0].duration, Some(Duration::from_millis(1234)));
        assert_eq!(results[1].name, "my-crate::tests::bar");
        assert_eq!(results[1].status, TestStatus::Fail);
        assert_eq!(results[2].name, "my-crate::tests::baz");
        assert_eq!(results[2].status, TestStatus::Fail); // timeout = fail
        assert_eq!(results[3].name, "my-crate::tests::qux");
        assert_eq!(results[3].status, TestStatus::Ignored);
        assert_eq!(results[3].duration, None);
    }

    #[test]
    fn test_duration_ms_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct T {
            #[serde(with = "duration_ms")]
            d: Duration,
        }
        let t = T {
            d: Duration::from_millis(1234),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#"{"d":1234}"#);
        let t2: T = serde_json::from_str(&json).unwrap();
        assert_eq!(t, t2);
    }

    #[test]
    fn test_option_duration_ms_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct T {
            #[serde(with = "option_duration_ms")]
            d: Option<Duration>,
        }
        let t = T {
            d: Some(Duration::from_millis(42)),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#"{"d":42}"#);
        let t2: T = serde_json::from_str(&json).unwrap();
        assert_eq!(t, t2);

        let none = T { d: None };
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, r#"{"d":null}"#);
        let t3: T = serde_json::from_str(&json).unwrap();
        assert_eq!(none, t3);
    }

    #[test]
    fn test_run_manifest_backward_compat() {
        // Old-style run.json with test_outcome instead of outcome
        let json = r#"{
            "kind": "sim",
            "test_outcome": "success",
            "project": "iroh"
        }"#;
        let m: RunManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.outcome.as_deref(), Some("success"));
        assert_eq!(m.kind, RunKind::Sim);
    }
}
