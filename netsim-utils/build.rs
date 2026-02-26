use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.parent().expect("workspace root").join("ui");
    println!("cargo:rerun-if-changed=../ui/package.json");
    println!("cargo:rerun-if-changed=../ui/package-lock.json");
    println!("cargo:rerun-if-changed=../ui/vite.config.ts");
    println!("cargo:rerun-if-changed=../ui/tsconfig.json");
    println!("cargo:rerun-if-changed=../ui/tsconfig.node.json");
    println!("cargo:rerun-if-changed=../ui/index.html");
    println!("cargo:rerun-if-changed=../ui/src");
    run_npm(&ui_dir, &["install"]);
    run_npm(&ui_dir, &["run", "build"]);
}

fn run_npm(ui_dir: &Path, args: &[&str]) {
    let status = Command::new("npm")
        .args(args)
        .current_dir(ui_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap_or_else(|err| panic!("failed to run npm {}: {err}", args.join(" ")));
    if !status.success() {
        panic!("npm {} failed with status {status}", args.join(" "));
    }
}
