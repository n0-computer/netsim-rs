pub mod common;
pub mod container;
pub mod qemu;
pub mod util;

pub use common::{RunVmArgs, TestVmArgs};

use clap::ValueEnum;

/// VM backend selection.
#[derive(Clone, Debug, ValueEnum)]
pub enum Backend {
    /// Auto-detect: prefer `container` on macOS Apple Silicon, fall back to QEMU.
    Auto,
    /// QEMU with a full Debian cloud image and SSH access.
    Qemu,
    /// Apple `container` CLI (macOS 26 + Apple Silicon only).
    Container,
}

pub fn default_test_target() -> String {
    if std::env::consts::ARCH == "aarch64" {
        "aarch64-unknown-linux-musl".to_string()
    } else {
        "x86_64-unknown-linux-musl".to_string()
    }
}

/// Resolve `Backend::Auto` into a concrete backend.
pub fn resolve_backend(b: Backend) -> Backend {
    match b {
        Backend::Auto => {
            if std::env::consts::OS == "macos"
                && std::env::consts::ARCH == "aarch64"
                && common::command_exists("container").unwrap_or(false)
            {
                Backend::Container
            } else {
                Backend::Qemu
            }
        }
        other => other,
    }
}
