# Running in a VM

patchbay requires Linux network namespaces, which means it cannot run
natively on macOS or Windows. The `patchbay-vm` crate solves this by
wrapping your simulations and tests in a QEMU Linux VM, giving you the
same experience on any development machine.

## Installing patchbay-vm

```bash
cargo install --git https://github.com/n0-computer/patchbay patchbay-vm
```

## Running simulations

The `run` command boots a VM (or reuses a running one), stages the
simulation files and binaries, and executes them inside the guest:

```bash
patchbay-vm run ./sims/iperf-baseline.toml
```

Results and logs are written to the work directory (`.patchbay-work/` by
default). You can pass multiple simulation files, and they run
sequentially in the same VM.

### Controlling the patchbay version

By default, `patchbay-vm` downloads the latest release of the patchbay
runner binary. You can pin a version, build from a Git ref, or point to a
local binary:

```bash
patchbay-vm run sim.toml --patchbay-version v0.10.0
patchbay-vm run sim.toml --patchbay-version git:main
patchbay-vm run sim.toml --patchbay-version path:/usr/local/bin/patchbay
```

### Binary overrides

If your simulation references custom binaries (test servers, protocol
implementations), you can stage them into the VM:

```bash
patchbay-vm run sim.toml --binary myserver:path:./target/release/myserver
```

The binary is copied into the guest's work directory and made available
at the path the simulation expects.

## Running tests

The `test` command cross-compiles your Rust tests for musl, stages the
test binaries in the VM, and runs them:

```bash
patchbay-vm test
patchbay-vm test --package patchbay
patchbay-vm test -- --test-threads=4
```

This is the recommended way to run patchbay integration tests on macOS.
The VM has all required tools pre-installed (nftables, iproute2, iperf3)
and unprivileged user namespaces enabled.

## VM lifecycle

The VM boots on first use and stays running between commands. Subsequent
`run` or `test` calls reuse the existing VM, which avoids the 30-60
second boot time on repeated invocations.

```bash
patchbay-vm up        # Boot the VM (or verify it is running)
patchbay-vm status    # Show VM state, SSH port, mount paths
patchbay-vm down      # Shut down the VM
patchbay-vm cleanup   # Remove stale sockets and PID files
```

You can also SSH into the guest directly for debugging:

```bash
patchbay-vm ssh -- ip netns list
patchbay-vm ssh -- nft list ruleset
```

## How it works

`patchbay-vm` downloads a Debian cloud image (cached in
`~/.local/share/patchbay/qemu-images/`), creates a COW disk backed by
it, and boots QEMU with cloud-init for initial provisioning. The guest
gets SSH access via a host-forwarded port (default 2222) and three shared
mount points:

| Guest path | Host path | Access | Purpose |
|------------|-----------|--------|---------|
| `/app` | Workspace root | Read-only | Source code and simulation files |
| `/target` | Cargo target dir | Read-only | Build artifacts |
| `/work` | Work directory | Read-write | Simulation output and logs |

File sharing uses virtiofs when available (faster, requires virtiofsd on
the host) and falls back to 9p. Hardware acceleration is auto-detected:
KVM on Linux, HVF on macOS, TCG emulation as a last resort.

## Configuration

All settings have sensible defaults. Override them through environment
variables when needed:

| Variable | Default | Description |
|----------|---------|-------------|
| `QEMU_VM_MEM_MB` | 4096 | Guest RAM in megabytes |
| `QEMU_VM_CPUS` | 4 | Guest CPU count |
| `QEMU_VM_SSH_PORT` | 2222 | Host port forwarded to guest SSH |
| `QEMU_VM_NAME` | patchbay-vm | VM instance name |
| `QEMU_VM_DISK_GB` | 40 | Disk size in gigabytes |

VM state lives in `.qemu-vm/<name>/` in your project directory. The disk
image uses COW backing, so it only consumes space for blocks that differ
from the base image.
