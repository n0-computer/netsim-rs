//! Integration test for `patchbay compare test`.
//! Copies the counter fixture into a temp git repo, makes two commits
//! with different PACKET_COUNT values, and runs compare between them.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

#[test]
#[ignore] // Slow: builds fixture crate from scratch in worktrees
fn compare_detects_regression() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let cli_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let patchbay_crate = cli_dir.parent().unwrap().join("patchbay");
    let fixture_dir = cli_dir.join("tests/fixtures/counter");

    // Copy fixture files into temp dir
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::copy(
        fixture_dir.join("tests/counter.rs"),
        dir.join("tests/counter.rs"),
    )
    .unwrap();

    // Copy Cargo.toml and replace the relative patchbay path with absolute
    let cargo_toml = std::fs::read_to_string(fixture_dir.join("Cargo.toml")).unwrap();
    let cargo_toml = cargo_toml.replace(
        "path = \"../../../../patchbay\"",
        &format!("path = \"{}\"", patchbay_crate.display()),
    );
    std::fs::write(dir.join("Cargo.toml"), cargo_toml).unwrap();

    // Commit 1: passing (PACKET_COUNT = 5)
    git(dir, &["init"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "passing"]);
    git(dir, &["tag", "v1"]);

    // Commit 2: regressing (PACKET_COUNT = 2, below THRESHOLD = 3)
    let src = std::fs::read_to_string(dir.join("tests/counter.rs")).unwrap();
    let regressed = src.replace("const PACKET_COUNT: u32 = 5;", "const PACKET_COUNT: u32 = 2;");
    std::fs::write(dir.join("tests/counter.rs"), regressed).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "regressing"]);
    git(dir, &["tag", "v2"]);

    // Run compare
    let patchbay_bin = env!("CARGO_BIN_EXE_patchbay");
    let output = Command::new(patchbay_bin)
        .args(["-v", "compare", "test", "v1", "v2"])
        .current_dir(dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("stdout:\n{stdout}");
    eprintln!("stderr:\n{stderr}");

    // Compare should detect the regression and exit non-zero
    assert!(
        !output.status.success(),
        "expected non-zero exit due to regression"
    );

    // stdout should contain the summary output
    assert!(stdout.contains("Compare:"), "missing Compare header");
    assert!(stdout.contains("Score:"), "missing Score line");

    // Find the two persisted run directories in .patchbay/work/
    let work = dir.join(".patchbay/work");
    assert!(work.exists(), ".patchbay/work dir not created");

    let run_dirs: Vec<_> = std::fs::read_dir(&work)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("run-"))
        .collect();
    assert_eq!(run_dirs.len(), 2, "expected 2 run directories, found {}", run_dirs.len());

    // Parse run.json from each directory
    let mut manifests: Vec<serde_json::Value> = run_dirs
        .iter()
        .map(|d| {
            let run_json = d.path().join("run.json");
            assert!(run_json.exists(), "run.json not found in {}", d.path().display());
            serde_json::from_str(&std::fs::read_to_string(&run_json).unwrap()).unwrap()
        })
        .collect();

    // Both should have kind: "test"
    for m in &manifests {
        assert_eq!(m["kind"], "test", "run.json should have kind 'test'");
        assert!(!m["dirty"].as_bool().unwrap_or(true), "run should not be dirty");
        assert!(m["commit"].is_string(), "run.json should have a commit SHA");
    }

    // Resolve expected SHAs for v1 and v2
    let v1_sha = {
        let out = Command::new("git")
            .args(["rev-parse", "v1"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let v2_sha = {
        let out = Command::new("git")
            .args(["rev-parse", "v2"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    // Sort manifests so left (v1) comes first
    manifests.sort_by_key(|m| m["commit"].as_str().unwrap() == v2_sha);
    let left_manifest = &manifests[0];
    let right_manifest = &manifests[1];

    assert_eq!(left_manifest["commit"].as_str().unwrap(), v1_sha, "left run should match v1 SHA");
    assert_eq!(right_manifest["commit"].as_str().unwrap(), v2_sha, "right run should match v2 SHA");

    // Left side: both tests pass (PACKET_COUNT=5 >= THRESHOLD=3)
    assert_eq!(left_manifest["pass"].as_u64().unwrap(), 2, "left should have 2 passes");
    assert_eq!(left_manifest["fail"].as_u64().unwrap(), 0, "left should have 0 failures");
    assert_eq!(left_manifest["total"].as_u64().unwrap(), 2);

    // Right side: udp_threshold fails (PACKET_COUNT=2 < THRESHOLD=3)
    assert_eq!(right_manifest["pass"].as_u64().unwrap(), 1, "right should have 1 pass");
    assert_eq!(right_manifest["fail"].as_u64().unwrap(), 1, "right should have 1 failure");
    assert_eq!(right_manifest["total"].as_u64().unwrap(), 2);

    // Per-test results
    let left_tests = left_manifest["tests"].as_array().unwrap();
    let right_tests = right_manifest["tests"].as_array().unwrap();
    assert_eq!(left_tests.len(), 2, "should have 2 left test results");
    assert_eq!(right_tests.len(), 2, "should have 2 right test results");

    // Find the threshold test in right results — it should fail
    let threshold_right = right_tests
        .iter()
        .find(|r| r["name"].as_str().unwrap().contains("udp_threshold"))
        .expect("udp_threshold test not found in right results");
    assert_eq!(threshold_right["status"], "fail");

    // Find the threshold test in left results — it should pass
    let threshold_left = left_tests
        .iter()
        .find(|r| r["name"].as_str().unwrap().contains("udp_threshold"))
        .expect("udp_threshold test not found in left results");
    assert_eq!(threshold_left["status"], "pass");

    // Worktrees should be cleaned up (no changes = removed)
    let tree_dir = dir.join(".patchbay/tree");
    if tree_dir.exists() {
        let remaining: Vec<_> = std::fs::read_dir(&tree_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            remaining.is_empty(),
            "worktrees should be cleaned up, found: {remaining:?}"
        );
    }

    // Validate metrics.jsonl from testdir output.
    // The fixture uses testdir!() so Lab output goes to
    // <target_dir>/testdir-current/<test>/device.sender.metrics.jsonl
    // Use cargo metadata to find the target dir in the temp repo.
    let testdir_current = {
        let meta_out = Command::new("cargo")
            .args(["metadata", "--format-version=1", "--no-deps"])
            .current_dir(dir)
            .output()
            .unwrap();
        let meta: serde_json::Value =
            serde_json::from_slice(&meta_out.stdout).unwrap_or_default();
        let target = meta["target_directory"]
            .as_str()
            .map(|s| Path::new(s).join("testdir-current"));
        target.unwrap_or_else(|| dir.join("target/testdir-current"))
    };
    if testdir_current.exists() {
        let metrics_files: Vec<_> = walkdir(&testdir_current)
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .map_or(false, |f| f.to_string_lossy().ends_with(".metrics.jsonl"))
            })
            .collect();
        assert!(
            !metrics_files.is_empty(),
            "expected metrics.jsonl files in {}, found none",
            testdir_current.display()
        );

        // At least one metrics file should contain packet_count
        let mut found_packet_count = false;
        for path in &metrics_files {
            let content = std::fs::read_to_string(path).unwrap();
            for line in content.lines() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(m) = val.get("m").and_then(|m| m.as_object()) {
                        if let Some(count) = m.get("packet_count").and_then(|v| v.as_f64()) {
                            found_packet_count = true;
                            assert!(
                                count == 5.0 || count == 2.0,
                                "unexpected packet_count value: {count}"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            found_packet_count,
            "no packet_count metric found in metrics files"
        );
    }
}

/// Recursively collect all file paths under a directory.
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
}
