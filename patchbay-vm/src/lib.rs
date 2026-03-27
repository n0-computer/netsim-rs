pub mod common;
pub mod container;
pub mod qemu;
pub mod util;

use clap::ValueEnum;
pub use common::{RunVmArgs, TestVmArgs};

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

/// VM backend operations.
pub trait VmOps {
    fn up(&self, recreate: bool) -> anyhow::Result<()>;
    fn down(&self) -> anyhow::Result<()>;
    fn status(&self) -> anyhow::Result<()>;
    fn cleanup(&self) -> anyhow::Result<()>;
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()>;
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()>;
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()>;
}

/// QEMU backend.
pub struct Qemu;

impl VmOps for Qemu {
    fn up(&self, recreate: bool) -> anyhow::Result<()> {
        qemu::up_cmd(recreate)
    }
    fn down(&self) -> anyhow::Result<()> {
        qemu::down_cmd()
    }
    fn status(&self) -> anyhow::Result<()> {
        qemu::status_cmd()
    }
    fn cleanup(&self) -> anyhow::Result<()> {
        qemu::cleanup_cmd()
    }
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()> {
        qemu::ssh_cmd_cli(cmd)
    }
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()> {
        qemu::run_sims_in_vm(args)
    }
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()> {
        qemu::run_tests_in_vm(args)
    }
}

/// Apple container backend.
pub struct Container;

impl VmOps for Container {
    fn up(&self, recreate: bool) -> anyhow::Result<()> {
        container::up_cmd(recreate)
    }
    fn down(&self) -> anyhow::Result<()> {
        container::down_cmd()
    }
    fn status(&self) -> anyhow::Result<()> {
        container::status_cmd()
    }
    fn cleanup(&self) -> anyhow::Result<()> {
        container::cleanup_cmd()
    }
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()> {
        container::exec_cmd_cli(cmd)
    }
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()> {
        container::run_sims(args)
    }
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()> {
        container::run_tests(args)
    }
}

impl Backend {
    /// Resolve `Auto` into a concrete backend.
    pub fn resolve(self) -> Self {
        match self {
            Self::Auto => {
                if std::env::consts::OS == "macos"
                    && std::env::consts::ARCH == "aarch64"
                    && common::command_exists("container").unwrap_or(false)
                {
                    Self::Container
                } else {
                    Self::Qemu
                }
            }
            other => other,
        }
    }
}

/// Implement VmOps on Backend by delegating to the resolved backend.
impl VmOps for Backend {
    fn up(&self, recreate: bool) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.up(recreate),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.up(recreate),
        }
    }
    fn down(&self) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.down(),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.down(),
        }
    }
    fn status(&self) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.status(),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.status(),
        }
    }
    fn cleanup(&self) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.cleanup(),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.cleanup(),
        }
    }
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.exec(cmd),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.exec(cmd),
        }
    }
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.run_sims(args),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.run_sims(args),
        }
    }
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()> {
        match self {
            Self::Container => Container.run_tests(args),
            Self::Auto => {
                anyhow::bail!("Backend::Auto must be resolved before use; call .resolve() first")
            }
            Self::Qemu => Qemu.run_tests(args),
        }
    }
}
