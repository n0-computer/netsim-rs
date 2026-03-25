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

impl Backend {
    /// Resolve auto-detection and return a concrete backend.
    pub fn resolve(self) -> Self {
        resolve_backend(self)
    }

    pub fn up(&self, recreate: bool) -> anyhow::Result<()> {
        match self {
            Self::Container => container::up_cmd(recreate),
            _ => qemu::up_cmd(recreate),
        }
    }

    pub fn down(&self) -> anyhow::Result<()> {
        match self {
            Self::Container => container::down_cmd(),
            _ => qemu::down_cmd(),
        }
    }

    pub fn status(&self) -> anyhow::Result<()> {
        match self {
            Self::Container => container::status_cmd(),
            _ => qemu::status_cmd(),
        }
    }

    pub fn cleanup(&self) -> anyhow::Result<()> {
        match self {
            Self::Container => container::cleanup_cmd(),
            _ => qemu::cleanup_cmd(),
        }
    }

    pub fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()> {
        match self {
            Self::Container => container::exec_cmd_cli(cmd),
            _ => qemu::ssh_cmd_cli(cmd),
        }
    }

    pub fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()> {
        match self {
            Self::Container => container::run_sims(args),
            _ => qemu::run_sims_in_vm(args),
        }
    }

    pub fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()> {
        match self {
            Self::Container => container::run_tests(args),
            _ => qemu::run_tests_in_vm(args),
        }
    }
}
