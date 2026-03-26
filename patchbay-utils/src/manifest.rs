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
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

/// Serialize/deserialize an `Option<Duration>` as integer milliseconds.
pub mod option_duration_ms {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

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
    let dirty = !Command::new("git")
        .args(["diff", "--quiet"])
        .status()
        .map(|s| s.success())
        .unwrap_or(true);
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

/// Parse `cargo test` / `cargo nextest` stdout into per-test results.
///
/// Recognises lines of the form:
/// - `test some::path ... ok`
/// - `test some::path ... FAILED`
/// - `test some::path ... ignored`
pub fn parse_test_output(output: &str) -> Vec<TestResult> {
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        let Some((name, status_str)) = rest.rsplit_once(" ... ") else {
            continue;
        };
        let status = match status_str.trim() {
            "ok" => TestStatus::Pass,
            "FAILED" => TestStatus::Fail,
            "ignored" => TestStatus::Ignored,
            _ => continue,
        };
        results.push(TestResult {
            name: name.trim().to_string(),
            status,
            duration: None,
        });
    }
    results
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
