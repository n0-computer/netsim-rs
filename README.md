# netsim-rs

A Linux network namespace simulator for writing repeatable network condition
tests. Define a topology and a sequence of steps in TOML, run a binary, get
structured results.

## Usage (Linux)

### Requirements

- Linux (bare metal or VM)
- `ip`, `tc`, `nft` in PATH
- Unprivileged user namespaces enabled. This is the default on most modern
  distros. If you are not sure:

  ```bash
  # Check:
  sysctl kernel.unprivileged_userns_clone

  # Enable (temporary):
  sudo sysctl -w kernel.unprivileged_userns_clone=1

  # Enable (permanent):
  echo 'kernel.unprivileged_userns_clone=1' | sudo tee /etc/sysctl.d/99-userns.conf
  sudo sysctl --system
  ```

  On systems using AppArmor (Ubuntu 24.04+), you may also need:

  ```bash
  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
  ```

### Installation

```bash
cargo install --git https://github.com/n0-computer/netsim-rs
```

### Usage

Run one sim file:

```bash
netsim run ./iroh-integration/netsim/sims/iperf-1to1-public.toml
```

Run all sims discovered from `netsim.toml` in the current directory (or parent):

```bash
netsim run
```

Prepare assets once, then run without rebuilding:

```bash
netsim prepare
netsim run --no-build
```

Serve the UI from a work directory:

```bash
netsim serve --work-dir .netsim-work
```

## Usage (MacOS)

Run sims inside the bundled QEMU Linux VM.

### Requirements

- macOS
- `qemu` installed via Homebrew:

```bash
brew install qemu
```

### Installation

```bash
cargo install --git https://github.com/n0-computer/netsim-rs netsim-vm
```

### Usage

Run one sim file:

```bash
netsim-vm run ./iroh-integration/netsim/sims/iperf-1to1-public.toml
```

Run all sims in a directory:

```bash
netsim-vm run ./iroh-integration/netsim/sims/
```

Stop the VM:

```bash
netsim-vm down
```

### Output files

Sim results land under `.netsim-work/` in the current directory:

```
.netsim-work/
  latest/                        # symlink to the most recent run
  <sim-name>-YYMMDD-HHMMSS/
    results.json
    results.md
    nodes/<device>/stdout.log
    nodes/<device>/stderr.log
  combined-results.json          # aggregated across all runs in this work root
  combined-results.md
```

## Architecture

netsim builds isolated network environments using Linux network namespaces. Each
simulated node (device or router) runs in its own namespace, which gives it a
private network stack, its own interfaces, routing table, and firewall rules.
Processes launched on a node inherit that namespace, so they behave as if they
were running on a separate machine.

**Topology construction.** When a sim starts, netsim reads the topology table
and calls `rtnetlink` (no external `ip` binary needed) to create virtual
ethernet pairs between namespaces, assign addresses, add routes, and configure
NAT rules with `nft`. This runs as the current user, without `sudo`, because
namespace operations run inside a user namespace.

**Link impairment.** After topology build, steps can apply `tc netem` and
`tc tbf` rules to introduce packet loss, delay, and rate limits on interfaces.
These run via the `tc` and `ip` binaries inside the target namespace.

**Step runner.** Steps execute sequentially. `spawn` steps launch processes in
the background while the runner continues. Pump threads tee stdout/stderr to
log files and can feed captures into a shared `CaptureStore`. Later steps can
reference captured values like `${id.capture_name}` with timeout handling.

**No root required.** The kernel allows namespace operations without elevated
privileges when `unprivileged_userns_clone` is enabled.

## Writing simulations

A simulation file defines three things: a topology, a set of binaries, and a
sequence of steps. Steps can spawn processes, run commands, wait for captures,
apply impairments, bring interfaces up or down, and assert on outputs.

The full syntax is documented in [docs/reference.md](docs/reference.md).

Quick example:

```toml
[sim]
name = "iperf-baseline-inline"

[[router]]
name = "relay"

[device.provider.eth0]
gateway = "relay"

[device.fetcher.eth0]
gateway = "relay"

[[step]]
action      = "spawn"
id          = "server"
device      = "provider"
cmd         = ["iperf3", "-s", "-1"]
ready_after = "1s"

[[step]]
action = "run"
id     = "client"
device = "fetcher"
parser = "json"
cmd    = ["iperf3", "-c", "$NETSIM_IP_provider", "-t", "10", "-J"]
[step.captures.bytes]
pick = ".end.sum_received.bytes"
[step.captures.duration_s]
pick = ".end.sum_received.seconds"
[step.results]
duration   = "client.duration_s"
down_bytes = "client.bytes"

[[step]]
action = "wait-for"
id     = "server"
```
