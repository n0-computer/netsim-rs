//! Integration test for `patchbay compare test`.
//! Creates a temp git repo with the counter fixture, makes two commits
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
#[ignore] // Requires namespace capabilities + builds from scratch
fn compare_detects_regression() {
    if patchbay::check_caps().is_err() {
        eprintln!("skipping: no namespace capabilities");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let patchbay_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let patchbay_crate = patchbay_root.join("patchbay");

    // Write workspace
    std::fs::write(
        dir.join("Cargo.toml"),
        "[workspace]\nmembers = [\"counter\"]\nresolver = \"2\"\n",
    )
    .unwrap();

    // Write counter crate
    let counter_dir = dir.join("counter");
    std::fs::create_dir_all(counter_dir.join("tests")).unwrap();
    std::fs::write(
        counter_dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"counter\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
            [dev-dependencies]\npatchbay = {{ path = \"{}\" }}\n\
            tokio = {{ version = \"1\", features = [\"rt\", \"macros\", \"net\", \"time\"] }}\n\
            anyhow = \"1\"\n",
            patchbay_crate.display()
        ),
    )
    .unwrap();

    // Read fixture source from our fixtures dir
    let fixture_src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/counter/tests/counter.rs"),
    )
    .unwrap();

    // Commit 1: passing (PACKET_COUNT = 5)
    std::fs::write(counter_dir.join("tests/counter.rs"), &fixture_src).unwrap();
    git(dir, &["init"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "passing"]);
    git(dir, &["tag", "v1"]);

    // Commit 2: regressing (PACKET_COUNT = 2, below THRESHOLD = 3)
    let regressed = fixture_src.replace(
        "const PACKET_COUNT: u32 = 5;",
        "const PACKET_COUNT: u32 = 2;",
    );
    std::fs::write(counter_dir.join("tests/counter.rs"), &regressed).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "regressing"]);
    git(dir, &["tag", "v2"]);

    // Run compare
    let patchbay_bin = env!("CARGO_BIN_EXE_patchbay");
    let output = Command::new(patchbay_bin)
        .args([
            "compare",
            "test",
            "--ref",
            "v1",
            "--ref2",
            "v2",
            "-p",
            "counter",
        ])
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

    // Find and parse the manifest
    let work = dir.join(".patchbay/work");
    let compare_dir = std::fs::read_dir(&work)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("compare-")
        })
        .expect("compare output dir not found");
    let manifest_path = compare_dir.path().join("summary.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();

    assert_eq!(manifest["left_ref"], "v1");
    assert_eq!(manifest["right_ref"], "v2");
    assert!(manifest["summary"]["regressions"].as_u64().unwrap() >= 1);
    assert!(manifest["summary"]["score"].as_i64().unwrap() < 0);
}
