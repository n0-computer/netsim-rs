//! ELF .init_array bootstrap — runs before main() and before tokio creates threads.
#[cfg(target_os = "linux")]
#[ctor::ctor]
fn userns_ctor() {
    // SAFETY: single-threaded ELF init context; raw libc only.
    unsafe { patchbay::init_userns_for_ctor() }
}
