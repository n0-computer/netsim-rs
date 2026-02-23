# netsim-rs

A Linux network namespace simulator for writing repeatable network condition
tests. Define a topology and a sequence of steps in TOML, run a binary, get
structured results.

## Usage

### Requirements

- Linux (bare metal or VM)
- `ip`, `tc`, `nft` in PATH
- Unprivileged user namespaces enabled -- this is the default on most modern
  distros. If you're not sure:

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

### Running a sim

The recommended way to run sims is inside the included QEMU VM, which handles
capability requirements automatically:

```bash
# Run a single sim
netsim run-vm ./iroh-integration/netsim/sims/iroh-1to1-nat.toml

# Run a whole directory and get combined results
netsim run-vm ./iroh-integration/netsim/sims/

# Keep a log
netsim run-vm ./iroh-integration/netsim/sims/iroh-1to1-nat.toml |& tee run.log

# Open the results in the UI after the run
netsim run-vm ./iroh-integration/netsim/sims/iroh-1to1-nat.toml --open
```

To test against a local iroh checkout with uncommitted changes:

```bash
netsim run-vm ./iroh-integration/netsim/sims/iroh-1to1-nat.toml \
  --binary "transfer:build:../iroh"
```

The `--binary` flag overrides any binary defined in the sim file:

```
--binary "<name>:<mode>:<value>"
```

| Mode    | Source |
|---------|--------|
| `build` | Build from a local checkout directory with `cargo build` |
| `fetch` | Download from a URL |
| `path`  | Copy a local binary into the work directory and use that |

#### Output files

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

Inside the VM the work root is `/work`, which maps to `.netsim-work/` on the host.

#### VM management

```bash
# Recreate the VM (needed when binary paths change)
netsim run-vm --recreate ./netsim/sims/iroh-1to1-nat.toml

# Shut down the VM
netsim vm-down

# Clean up leaked kernel resources from aborted runs
netsim cleanup
```

---

## Architecture

netsim builds isolated network environments using Linux network namespaces. Each
simulated node (device or router) runs in its own namespace, which gives it a
completely private network stack -- its own interfaces, routing table, and
firewall rules. Processes launched on a node inherit that namespace, so they
behave exactly as if they were running on a separate machine.

**Topology construction.** When a sim starts, netsim reads the topology table
and calls `rtnetlink` (no external `ip` binary needed) to create virtual ethernet
pairs between namespaces, assign addresses, add routes, and configure NAT rules
with `nft`. All of this runs as the current user, without `sudo`, because the
namespace operations are performed inside a user namespace (each namespace is
owned by the process's user).

**Link impairment.** After the topology is built, steps can apply `tc netem`
and `tc tbf` rules to introduce packet loss, delay, and rate limits on
individual interfaces. These run via the `tc` and `ip` binaries inside the
appropriate namespace.

**The step runner.** Steps execute sequentially in a single loop. `spawn` steps
launch processes in the background; the runner continues to the next step while
those processes run. Pump threads tee the stdout/stderr of each process to log
files and optionally forward lines to a capture reader thread, which applies
regex or JSON patterns and writes matches to a shared `CaptureStore`. When a
later step references `${id.capture_name}`, it blocks on the `CaptureStore`
condvar until the value appears or a timeout fires.

**No root required.** The user namespace trick means the kernel allows creating
and configuring namespaces without elevated privileges, as long as
`unprivileged_userns_clone` is enabled. The VM path adds another layer: a QEMU
microVM handles the namespace work inside a proper Linux guest, which is useful
when the host OS or container environment restricts namespace creation.

---

## Writing simulations

A simulation file defines three things: a topology, a set of binaries, and a
sequence of steps. Steps can spawn processes, run commands, wait for captures,
apply impairments, bring interfaces up or down, and assert on outputs.

The full syntax is documented in [docs/reference.md](docs/reference.md).

Quick example:

```toml
[sim]
name     = "iperf-baseline"
topology = "1to1-public"

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
[step.results]
down_bytes = "client.bytes"

[[step]]
action = "wait-for"
id     = "server"

[[step]]
action = "assert"
checks = ["client.bytes matches [0-9]+"]
```

The included iroh integration lives in `iroh-integration/`:

```
iroh-integration/
  iroh-defaults.toml            # shared binary defs, relay/transfer templates
  topos/                        # topology files
  sims/
    iroh-1to1-public.toml       # direct transfer, two public nodes
    iroh-1to1-nat.toml          # transfer through NAT, relay-assisted
    iroh-1to1-nat-switch.toml   # route switch mid-transfer test
    iroh-1to10-public.toml      # 1 provider, 10 concurrent fetchers
    iperf-1to1-public.toml      # iperf3 baseline on a public topology
```
