/// Build RUSTFLAGS with --cfg patchbay_tests appended.
pub fn patchbay_rustflags() -> String {
    let existing = std::env::var("RUSTFLAGS").unwrap_or_default();
    if existing.is_empty() {
        "--cfg patchbay_tests".to_string()
    } else {
        format!("{existing} --cfg patchbay_tests")
    }
}
