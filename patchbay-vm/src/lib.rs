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

/// Backend operations for VM-based execution.
pub trait VmOps {
    fn up(&self, recreate: bool) -> anyhow::Result<()>;
    fn down(&self) -> anyhow::Result<()>;
    fn status(&self) -> anyhow::Result<()>;
    fn cleanup(&self) -> anyhow::Result<()>;
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()>;
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()>;
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()>;
}

pub struct QemuBackend;
pub struct ContainerBackend;

impl VmOps for QemuBackend {
    fn up(&self, recreate: bool) -> anyhow::Result<()> { qemu::up_cmd(recreate) }
    fn down(&self) -> anyhow::Result<()> { qemu::down_cmd() }
    fn status(&self) -> anyhow::Result<()> { qemu::status_cmd() }
    fn cleanup(&self) -> anyhow::Result<()> { qemu::cleanup_cmd() }
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()> { qemu::ssh_cmd_cli(cmd) }
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()> { qemu::run_sims_in_vm(args) }
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()> { qemu::run_tests_in_vm(args) }
}

impl VmOps for ContainerBackend {
    fn up(&self, recreate: bool) -> anyhow::Result<()> { container::up_cmd(recreate) }
    fn down(&self) -> anyhow::Result<()> { container::down_cmd() }
    fn status(&self) -> anyhow::Result<()> { container::status_cmd() }
    fn cleanup(&self) -> anyhow::Result<()> { container::cleanup_cmd() }
    fn exec(&self, cmd: Vec<String>) -> anyhow::Result<()> { container::exec_cmd_cli(cmd) }
    fn run_sims(&self, args: RunVmArgs) -> anyhow::Result<()> { container::run_sims(args) }
    fn run_tests(&self, args: TestVmArgs) -> anyhow::Result<()> { container::run_tests(args) }
}

/// Resolve backend and return a boxed trait object.
pub fn resolve_ops(b: Backend) -> Box<dyn VmOps> {
    let resolved = resolve_backend(b);
    match resolved {
        Backend::Container => Box::new(ContainerBackend),
        _ => Box::new(QemuBackend),
    }
}
