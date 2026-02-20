# Plan: Iroh Netsims (v4)

Goal: `cargo run -- sims/iroh-1to1.toml` builds a network, runs an iroh transfer
sim, applies scheduled network events, and reports results.

Reference: `resources/chuck/netsim/` — the Python/mininet implementation we're
porting. `resources/dogfood/` — Rust transfer logic to port.

---

## Status: DRAFT v4

**Key changes from v3:**
- Process log structure and result parsing added (were entirely missing)
- `kind = "iroh-transfer"` sequence corrected: `PathStats` = end-of-connection on
  provider side (not a metric, not readiness); `connected_via` comes from
  `ConnectionTypeChanged` events; `--output json --logs-path` flags documented
- `[binary]` gains `url` source option; `[relay_binary]` section added
- `wait-for` default timeout specified: 300 s
- `count` restored (Phase 3, needed for 1→N sims)
- Capture→substitution dependency ordering documented
- IROH_DATA_DIR dropped (no longer needed in modern iroh)

---

## 1. Router Format — Unified `[[router]]`

All topology nodes are `[[router]]`. No more `[[isp]]` / `[[dc]]` / `[[lan]]`.

```toml
[[router]]
name   = "dc-eu"
region = "eu"
# no upstream → IX-attached; public downstream pool; no NAT

[[router]]
name   = "isp-eu"
region = "eu"
nat    = "cgnat"
# cgnat → private downstream pool; SNAT on IX interface

[[router]]
name     = "lan-eu"
upstream = "isp-eu"
nat      = "destination-independent"
# upstream set → subscriber link into isp-eu's bridge
```

### Field semantics

| Field               | Values                                                                            | Default                                    |
|---------------------|-----------------------------------------------------------------------------------|--------------------------------------------|
| `name`              | string                                                                            | required                                   |
| `region`            | string (for inter-region latency)                                                 | none                                       |
| `upstream`          | router name                                                                       | none → connects to IX bridge               |
| `nat`               | `"none"` / `"cgnat"` / `"destination-independent"` / `"destination-dependent"`   | `"none"`                                   |
| `alloc_global_ipv4` | bool — downstream pool selection                                                  | `true` if no upstream; `false` otherwise   |

**`nat` values:**
- `"none"` — no NAT; downstream addresses are publicly routable (DC behaviour)
- `"cgnat"` — SNAT subscriber traffic on the IX-facing interface
- `"destination-independent"` — EIM home NAT (`snat … persistent`)
- `"destination-dependent"` — EDM/symmetric home NAT (`masquerade random`)

**`alloc_global_ipv4`:** inferred from `nat`; only override when the default is
wrong. Drop from TOML for v1.

### Rust API

```rust
impl Lab {
    pub fn add_router(
        &mut self,
        name: &str,
        region: Option<&str>,
        upstream: Option<NodeId>,
        nat: NatMode,
    ) -> Result<NodeId>;
}

pub enum NatMode {
    None,
    Cgnat,
    DestinationIndependent,
    DestinationDependent,
}
```

`add_router` infers `alloc_global_ipv4` and `cgnat` from `nat`.

---

## 2. Device Format — Per-Interface Tables

Each interface is declared as a sub-table `[device.<name>.<ifname>]`.
Device-level settings go in `[device.<name>]` (optional).

```toml
# Simple: single interface
[device.provider.eth0]
gateway = "dc-eu"

# Multi-interface: two pre-wired uplinks
[device.fetcher]
default_via = "eth1"          # eth1 is active at startup

[device.fetcher.eth0]
gateway = "isp-eu"
impair  = "mobile"

[device.fetcher.eth1]
gateway = "lan-eu"
impair  = "wifi"
```

### Field semantics

`[device.<name>]` (device-level, optional):

| Field        | Values              | Default                     |
|--------------|---------------------|-----------------------------|
| `default_via`| interface name      | first interface encountered |
| `count`      | integer ≥ 1         | 1 (Phase 3)                 |

`[device.<name>.<ifname>]` (per-interface):

| Field     | Values                                          | Default  |
|-----------|-------------------------------------------------|----------|
| `gateway` | router name                                     | required |
| `impair`  | `"wifi"` / `"mobile"` / `{ rate, loss, latency }` | none  |

### `count` shorthand (Phase 3)

```toml
[device.fetcher]
count = 3

[device.fetcher.eth0]
gateway = "dc-eu"
impair  = { loss = 1, latency = 200, rate_kbit = 1000 }
```

Creates `fetcher-0`, `fetcher-1`, `fetcher-2`, each with one `eth0`.
Env vars: `$NETSIM_IP_fetcher_0`, `$NETSIM_IP_fetcher_1`, etc.
Only valid when all interfaces are identical (single-template expansion).

### TOML parsing note

Parse `device` as `HashMap<String, toml::Value>` (raw), then post-process:
- String/integer values at device level → device-level config
- Table values → interface definitions (key = ifname)

---

## 3. Environment Variables

Every process started in a step receives:

| Variable                        | Value                                           |
|---------------------------------|-------------------------------------------------|
| `NETSIM_IP_<device>`            | IP of the `default_via` interface               |
| `NETSIM_IP_<device>_<ifname>`   | IP of the named interface                       |
| `NETSIM_IP_<device>_<N>`        | IP of the Nth `count`-expanded device (Phase 3) |
| `NETSIM_NS_<device>`            | netns name (one namespace per device always)    |

Variable name normalisation: device/interface names have `-` → `_`, uppercased.

Additionally, chuck-compatible variables set for every process:

| Variable       | Value                                         |
|----------------|-----------------------------------------------|
| `RUST_LOG_STYLE` | `never` (disables ANSI colour; required for NDJSON parsing) |
| `RUST_LOG`     | `warn,iroh::_events::conn_type=trace` unless caller sets it |
| `SSLKEYLOGFILE`| `<work_dir>/logs/keylog_<step_id>.txt`        |

User can override any of these via `env = { KEY = "value" }` on the step.

---

## 4. Rust Types — Device

```rust
// In core.rs

pub struct Device {
    pub id:          DeviceId,
    pub name:        String,
    pub ns:          String,
    pub interfaces:  Vec<DeviceIface>,  // in declaration order
    pub default_via: String,            // ifname of the active default route
}

pub struct DeviceIface {
    pub ifname: String,
    pub uplink: SwitchId,
    pub ip:     Option<Ipv4Addr>,
    pub impair: Option<Impair>,
}

impl Device {
    pub fn iface(&self, name: &str) -> Option<&DeviceIface> {
        self.interfaces.iter().find(|i| i.ifname == name)
    }
    pub fn default_iface(&self) -> &DeviceIface {
        self.iface(&self.default_via).expect("default_via is valid")
    }
}
```

Old `Device` fields `uplink`, `ip`, `impair_upstream` are removed.

---

## 5. Rust API — DeviceBuilder

```rust
impl Lab {
    /// Start building a device.  Call `.iface(…)` one or more times, then `.build()`.
    pub fn add_device(&mut self, name: &str) -> DeviceBuilder<'_>;
}

pub struct DeviceBuilder<'a> {
    lab:         &'a mut Lab,
    name:        String,
    interfaces:  Vec<IfaceCfg>,
    default_via: Option<String>,
}

struct IfaceCfg {
    ifname:  String,
    gateway: NodeId,
    impair:  Option<Impair>,
}

impl<'a> DeviceBuilder<'a> {
    pub fn iface(mut self, ifname: &str, gateway: NodeId, impair: Option<Impair>) -> Self;
    pub fn default_via(mut self, ifname: &str) -> Self;
    pub fn build(self) -> Result<NodeId>;
}
```

Usage:

```rust
let provider = lab.add_device("provider")
    .iface("eth0", dc_eu, None)
    .build()?;

let fetcher = lab.add_device("fetcher")
    .iface("eth0", isp_eu, Some(Impair::Mobile))
    .iface("eth1", lan_eu, Some(Impair::Wifi))
    .default_via("eth1")
    .build()?;
```

`add_isp`, `add_dc`, `add_home`, and the `Gateway` enum are **removed**.
All existing tests updated to use `add_router` + `DeviceBuilder`.

---

## 6. Build Changes — Multi-Interface Wiring

`DevBuild` → `IfaceBuild` (one record per device-interface pair).
`wire_device` → `wire_iface`.

```rust
struct IfaceBuild {
    dev_ns:     String,
    gw_ns:      String,
    gw_ip:      Ipv4Addr,
    gw_br:      String,
    dev_ip:     Ipv4Addr,
    prefix_len: u8,
    impair:     Option<Impair>,
    ifname:     String,      // "eth0", "eth1", …
    is_default: bool,        // only this interface gets `ip route add default`
    idx:        u64,         // globally unique; drives veth naming
}
```

`wire_iface` identical to current `wire_device` except:
- Rename the device-side veth to `dev.ifname` instead of always `"eth0"`.
- Only call `add_default_route_v4` when `is_default == true`.

`LabCore::build` collects one `IfaceBuild` per `(device, interface)` pair and
calls `wire_iface` for each.

---

## 7. Dynamic Network Operations

### 7a. `qdisc::remove_qdisc_r`

The existing `remove_qdisc` silently swallows errors.  Add a fallible version:

```rust
pub(crate) fn remove_qdisc_r(ns: &str, ifname: &str) -> Result<()> {
    let status = run_in_netns(ns, {
        let mut cmd = Command::new("tc");
        cmd.args(["qdisc", "del", "dev", ifname, "root"]);
        cmd.stderr(Stdio::null());
        cmd
    })?;
    // exit code 2 = ENOENT (no such qdisc) — acceptable
    if !status.success() && status.code() != Some(2) {
        bail!("tc qdisc del failed on {} in {}", ifname, ns);
    }
    Ok(())
}
```

### 7b. `Lab::set_impair`

```rust
impl Lab {
    pub fn set_impair(
        &mut self,
        device: &str,
        ifname: Option<&str>,   // None → default_via
        impair: Option<Impair>, // None → remove
    ) -> Result<()>;
}
```

### 7c. `Lab::link_down` / `Lab::link_up`

```rust
impl Lab {
    pub fn link_down(&mut self, device: &str, ifname: &str) -> Result<()>;
    pub fn link_up  (&mut self, device: &str, ifname: &str) -> Result<()>;
}
```

`ip link set <ifname> down/up` via `with_netns_thread`.

### 7d. `Lab::switch_route`

```rust
impl Lab {
    pub fn switch_route(&mut self, device: &str, to: &str) -> Result<()>;
}
```

`to` is always an explicit interface name (`"eth0"`, `"eth1"`).

Implementation:
1. Resolve `iface = dev.iface(to)`.
2. In device netns: `ip route del default` then `ip route add default via <gw_ip> dev <ifname>`.
3. Re-apply impairment on the newly active interface (or remove if none).
4. Update `dev.default_via = to.to_string()`.

`LabCore` additions needed:
```rust
pub fn router_downlink_gw_for_switch(&self, sw: SwitchId) -> Result<Ipv4Addr>;
pub fn set_device_default_via(&mut self, name: &str, ifname: &str) -> Result<()>;
```

All dynamic-op methods take `&mut self` — the sim runner holds exclusive access.

---

## 8. Process Logs

Every `spawn` and `run` step tees the process's stdout+stderr to a log file
while also streaming live for `ready_when` / `captures` matching.

Directory layout under `<work_dir>/`:

```
<work_dir>/
  logs/
    <step_id>.log          # stdout+stderr of the process
    keylog_<step_id>.txt   # SSLKEYLOGFILE
  results.json             # written at end of sim
```

`step_id` is the `id` field on `spawn` steps, or `<action>_<device>` for
unnamed `run` steps.

For `kind = "iroh-transfer"`, both sub-processes get their own log files:

```
logs/
  xfer_provider.log
  xfer_fetcher.log          # (or xfer_fetcher_0.log, xfer_fetcher_1.log for count > 1)
  keylog_xfer_provider.txt
  keylog_xfer_fetcher.txt
```

---

## 9. Result Parsing and Reporting

After all steps complete, the runner post-processes logs and writes
`<work_dir>/results.json`.

### Transfer stats (iroh)

Scan the fetcher log for the `DownloadComplete` NDJSON event:

```json
{"kind": "DownloadComplete", "size": 1073741824, "duration": 12345678}
```

- `size` = bytes transferred
- `duration` = microseconds
- Derived: `elapsed_s = duration / 1e6`, `mbps = size * 8 / (elapsed_s * 1e6)`

### Connection type (iroh)

Scan the fetcher log for `ConnectionTypeChanged` events:

```json
{"kind": "ConnectionTypeChanged", "status": "Selected", "addr": "Ip(...)"}
```

- Collect all events where `status == "Selected"`.
- The **last** such event determines `final_conn_direct`:
  - `addr` containing `"Ip"` → direct
  - anything else (relay URL) → relay
- Also capture `conn_upgrade` (ever went direct) and `conn_events` (total count).

### Output format

```json
{
  "sim": "iroh-1to1",
  "transfers": [
    {
      "id":               "xfer",
      "provider":         "provider",
      "fetcher":          "fetcher",
      "size_bytes":       1073741824,
      "elapsed_s":        12.345,
      "mbps":             695.4,
      "final_conn_direct": true,
      "conn_upgrade":     true,
      "conn_events":      2
    }
  ]
}
```

### `parser` field on generic `spawn` steps

```toml
[[step]]
action = "spawn"
id     = "get"
device = "fetcher"
cmd    = ["${binary}", "--output", "json", "fetch", "${srv.endpoint_id}"]
parser = "iroh_json"   # post-process log for DownloadComplete after step exits
```

Supported parsers for generic steps:

| `parser`    | Extracts                                 |
|-------------|------------------------------------------|
| `iroh_json` | `DownloadComplete` + `ConnectionTypeChanged` |
| `iperf`     | iperf throughput lines                   |
| none        | no post-processing                       |

---

## 10. Sim File Format

```toml
[sim]
name     = "iroh-wifi-to-mobile"
topology = "wifi-to-mobile"     # loads topos/wifi-to-mobile.toml

[binary]
repo    = "https://github.com/n0-computer/iroh"
commit  = "main"
example = "transfer"

[relay_binary]
url = "https://github.com/n0-computer/iroh/releases/download/v0.35.0/iroh-relay-x86_64-unknown-linux-musl.tar.gz"
```

Inline topology (no `topology` ref):

```toml
[sim]
name = "ping-basic"

[[router]]
name   = "dc-eu"
region = "eu"

[device.server.eth0]
gateway = "dc-eu"

[device.client.eth0]
gateway = "dc-eu"

[[step]]
action = "run"
device = "client"
cmd    = ["ping", "-c4", "$NETSIM_IP_server"]
```

When `topology` is set, router/device tables are loaded from
`topos/<topology>.toml` and must not appear inline.

---

## 11. Binary Spec

### `[binary]` — transfer binary

Three mutually exclusive sources:

```toml
# Option A: build from git
[binary]
repo    = "https://github.com/n0-computer/iroh"
commit  = "main"     # branch, tag, or full SHA
example = "transfer" # cargo --example <name>

# Option B: download from URL (tar.gz or bare binary)
[binary]
url = "https://github.com/n0-computer/iroh/releases/download/v0.35.0/iroh-x86_64-unknown-linux-musl.tar.gz"

# Option C: local prebuilt path
[binary]
path = "/usr/local/bin/iroh-transfer"
```

Build function (`src/sim/build.rs`):
- **git**: clone if no `.git`; `git fetch + checkout`; `cargo build --example … --release`.
  Skip build if binary mtime > source mtime.
- **url**: download + extract to `<work_dir>/bins/`; skip if already present.
- **path**: use as-is.

### `[relay_binary]` — relay binary (optional)

Same three sources plus inference:

```toml
# Option A: GitHub releases download (recommended)
[relay_binary]
url = "https://github.com/n0-computer/iroh/releases/download/v0.35.0/iroh-relay-x86_64-unknown-linux-musl.tar.gz"

# Option B: local path
[relay_binary]
path = "/usr/local/bin/iroh-relay"

# Option C: build from same repo/commit as [binary] (no extra field needed)
[relay_binary]
binary = "iroh-relay"   # cargo --bin <name>; inherits repo/commit from [binary]

# If [relay_binary] is omitted entirely: look for `iroh-relay` on $PATH.
```

Binary substitution variable available in steps: `${relay_binary}`.

---

## 12. Step Actions

| `action`       | Required fields                                | Optional                                  |
|----------------|------------------------------------------------|-------------------------------------------|
| `spawn`        | `id`, `device`, `cmd` **or** `kind`            | `ready_when`, `ready_after`, `captures`, `parser`, `env` |
| `run`          | `device`, `cmd`                                | `env`                                     |
| `wait`         | `duration`                                     | —                                         |
| `wait-for`     | `id`                                           | `timeout` (default: `"300s"`)             |
| `set-impair`   | `device`, `impair`                             | `interface` (default: `default_via`)      |
| `switch-route` | `device`, `to`                                 | —                                         |
| `link-down`    | `device`, `interface`                          | —                                         |
| `link-up`      | `device`, `interface`                          | —                                         |
| `assert`       | `check`                                        | `timeout`                                 |

`ready_after = "2s"` — static delay before the step is considered ready
(useful for relay nodes that don't emit a startup event).

`captures` — block on stdout until a regex matches, extract a named group:

```toml
captures = { addr = { stdout_regex = "READY (.+)" } }
```

Captured values are available as `${<id>.<name>}` in later steps.

**Capture→substitution ordering:** `${<id>.<capture>}` in a step's `cmd` is
interpolated at execution time.  The referenced `id` must have already run and
its `captures` resolved before the current step executes.  Steps execute
sequentially, so placing the dependent step after its source in TOML is
sufficient.

### `kind = "iroh-transfer"`

```toml
[[step]]
action     = "spawn"
kind       = "iroh-transfer"
id         = "xfer"
provider   = "provider"          # device name
fetcher    = "fetcher"           # single device  — or —
# fetchers = ["f-0", "f-1"]     # multiple devices (count-expanded; Phase 3)
relay_url  = "http://..."        # optional; passed as --relay-url to both sides
fetch_args = ["--verify"]        # optional extra args for fetcher(s)
```

#### Binary invocation

```
# Provider (inside provider's netns):
<binary> --output json --logs-path <log_dir>/xfer_provider provide

# Fetcher (inside fetcher's netns):
<binary> --output json --logs-path <log_dir>/xfer_fetcher \
         fetch <endpoint_id> [--relay-url <url>] [fetch_args…]
```

`--output json` switches the binary to NDJSON stdout.  Without this flag the
binary emits human-readable text that cannot be parsed.

#### Execution sequence

1. Start provider subprocess in provider's netns.
2. Stream provider stdout → write to `xfer_provider.log`.
3. Block until `{"kind":"EndpointBound","endpoint_id":"…"}` — extract
   `endpoint_id`.  Expose as `${xfer.endpoint_id}`.
4. Start fetcher subprocess(es) in their netns(es) with `endpoint_id`.
5. Stream fetcher stdout → write to `xfer_fetcher.log`.
6. Block until fetcher emits its own `EndpointBound` (confirms it started).
7. **Concurrently:**
   - Fetcher side: wait for fetcher process to exit naturally (exits after
     `DownloadComplete`).
   - Provider side: stream remaining stdout until `{"kind":"PathStats"}` —
     this is the provider's **end-of-connection** signal (emitted when the
     peer disconnects after the transfer).  Then SIGINT the provider and drain
     its remaining stdout.
8. Post-process `xfer_fetcher.log`:
   - Extract `DownloadComplete` → size, duration → Mbps.
   - Collect `ConnectionTypeChanged` events → `final_conn_direct`,
     `conn_upgrade`, `conn_events`.
9. Write results to `results.json`.

Exposes for `assert`:
- `xfer.mbps`, `xfer.elapsed_s`, `xfer.size_bytes`
- `xfer.final_conn_direct` (bool), `xfer.conn_upgrade` (bool), `xfer.conn_events` (int)
- `xfer.endpoint_id` (string)

### Variable substitution in `cmd`

- `$NETSIM_IP_<device>` — default-via IP
- `$NETSIM_IP_<device>_<ifname>` — specific interface IP
- `$NETSIM_NS_<device>` — netns name
- `${binary}` — path to built/downloaded transfer binary
- `${relay_binary}` — path to relay binary
- `${data}` — sim-specific data directory (`<work_dir>/data/`)
- `${<id>.<capture>}` — value captured from a prior `spawn`

---

## 13. Example Sim Files

### `sims/iroh-1to1.toml` — both public, no NAT

```toml
[sim]
name     = "iroh-1to1"
topology = "1to1-public"

[binary]
url = "https://github.com/n0-computer/iroh/releases/download/v0.35.0/iroh-transfer-x86_64-unknown-linux-musl.tar.gz"

[[step]]
action   = "spawn"
kind     = "iroh-transfer"
id       = "xfer"
provider = "provider"
fetcher  = "fetcher"

[[step]]
action  = "wait-for"
id      = "xfer"

[[step]]
action = "assert"
check  = "xfer.final_conn_direct == true"
```

### `sims/iroh-1to1-nat-both.toml` — both behind NAT + relay

```toml
[sim]
name     = "iroh-1to1-nat-both"
topology = "1to1-nat-both"

[binary]
url = "..."

[relay_binary]
url = "https://github.com/n0-computer/iroh/releases/download/v0.35.0/iroh-relay-x86_64-unknown-linux-musl.tar.gz"

[[step]]
action      = "spawn"
id          = "relay"
device      = "relay"
cmd         = ["${relay_binary}", "--dev"]
ready_after = "2s"

[[step]]
action    = "spawn"
kind      = "iroh-transfer"
id        = "xfer"
provider  = "provider"
fetcher   = "fetcher"
relay_url = "http://$NETSIM_IP_relay:3340"

[[step]]
action = "wait-for"
id     = "xfer"

[[step]]
action = "assert"
check  = "xfer.final_conn_direct == true"
```

### `sims/iroh-wifi-to-mobile.toml` — mid-transfer route switch

```toml
[sim]
name     = "iroh-wifi-to-mobile"
topology = "wifi-to-mobile"

[binary]
url = "..."

[[step]]
action   = "spawn"
kind     = "iroh-transfer"
id       = "xfer"
provider = "provider"
fetcher  = "fetcher"

[[step]]
action   = "wait"
duration = "10s"

[[step]]
action = "switch-route"
device = "fetcher"
to     = "eth0"        # switch from wifi (eth1) to mobile (eth0)

[[step]]
action  = "wait-for"
id      = "xfer"
timeout = "600s"

[[step]]
action = "assert"
check  = "xfer.final_conn_direct == true"
```

---

## 14. Topology Files

### `topos/1to1-public.toml`

```toml
[region.eu]
latencies = { us = 80 }
[region.us]
latencies = { eu = 80 }

[[router]]
name   = "dc-eu"
region = "eu"

[[router]]
name   = "dc-us"
region = "us"

[device.provider.eth0]
gateway = "dc-eu"

[device.fetcher.eth0]
gateway = "dc-eu"
```

### `topos/1to1-nat-both.toml`

```toml
[[router]]
name   = "dc-eu"
region = "eu"

[[router]]
name   = "isp-eu"
region = "eu"
nat    = "cgnat"

[[router]]
name     = "lan-provider"
upstream = "isp-eu"
nat      = "destination-independent"

[[router]]
name     = "lan-fetcher"
upstream = "isp-eu"
nat      = "destination-dependent"

[device.relay.eth0]
gateway = "dc-eu"

[device.provider.eth0]
gateway = "lan-provider"

[device.fetcher.eth0]
gateway = "lan-fetcher"
```

### `topos/wifi-to-mobile.toml`

```toml
[[router]]
name   = "dc-eu"
region = "eu"

[[router]]
name   = "isp-eu"
region = "eu"
nat    = "cgnat"

[[router]]
name     = "lan-eu"
upstream = "isp-eu"
nat      = "destination-independent"

[device.provider.eth0]
gateway = "dc-eu"

[device.fetcher]
default_via = "eth1"      # start on wifi

[device.fetcher.eth0]
gateway = "isp-eu"
impair  = "mobile"

[device.fetcher.eth1]
gateway = "lan-eu"
impair  = "wifi"
```

---

## 15. Module Layout

```
src/
  lib.rs          — Lab, DeviceBuilder, add_router, TOML config parsing
  core.rs         — LabCore, Device/DeviceIface, Netlink, build
  qdisc.rs        — tc helpers; add remove_qdisc_r
  sim/
    mod.rs        — SimConfig, run_sim
    topology.rs   — TopoConfig (shared between standalone topos/ and inline)
    build.rs      — build_or_fetch_binary (git / url / path)
    transfer.rs   — iroh-transfer kind (ported from resources/dogfood/)
    runner.rs     — step executor
    env.rs        — env-var injection + ${} interpolation
    report.rs     — result parsing (DownloadComplete, ConnectionTypeChanged) + results.json
  main.rs         — CLI entry point

sims/
  iroh-1to1.toml
  iroh-1to1-nat.toml
  iroh-1to1-nat-both.toml
  iroh-wifi-to-mobile.toml

topos/
  1to1-public.toml
  1to1-nat.toml
  1to1-nat-both.toml
  wifi-to-mobile.toml
```

---

## 16. CLI

```
cargo run -- sims/iroh-1to1.toml [--work-dir .netsim-work] [--set key=value …]
```

```rust
#[derive(Parser)]
struct Cli {
    sim:       PathBuf,
    #[arg(long, default_value = ".netsim-work")]
    work_dir:  PathBuf,
    #[arg(long = "set", value_parser = parse_kv)]
    overrides: Vec<(String, String)>,
}
```

`--set` applies dotted-key overrides to any scalar in the sim config.

---

## 17. Cargo.toml additions

```toml
[dependencies]
clap        = { version = "4", features = ["derive"] }
serde_json  = "1"
tokio       = { version = "1", features = ["full"] }
regex       = "1"
reqwest     = { version = "0.12", features = ["blocking"] }   # binary URL download
flate2      = "1"                                              # .tar.gz extraction
tar         = "0.4"
# iroh = "0.35"   # add when implementing the transfer kind (for EndpointId type)
```

---

## 18. Implementation Order

### Phase 1 — Core types + builder (all existing tests stay green)

1. **`core.rs`**: `Device` → multi-iface version; add `DeviceIface`; add
   `router_downlink_gw_for_switch`, `set_device_default_via`.
2. **`lib.rs`**: `DeviceBuilder`; remove `add_isp`/`add_dc`/`add_home`/`Gateway`;
   add `add_router`; update all tests.
3. **`core.rs` build**: `DevBuild` → `IfaceBuild`; `wire_device` → `wire_iface`;
   loop over all interfaces; only add default route for `default_via`.
4. **`lib.rs` TOML config**: parse `[[router]]` + `[device.name.ethN]`; remove
   old section parsing; update `load_from_toml` test.

### Phase 2 — Dynamic ops

5. **`qdisc.rs`**: add `remove_qdisc_r`.
6. **`lib.rs`**: `Lab::set_impair`, `Lab::link_down`, `Lab::link_up`,
   `Lab::switch_route` (all `&mut self`).
7. **Tests**: `set_impair` (RTT changes), `link_down/up` (connectivity),
   `switch_route` (RTT change after switch).

### Phase 3 — Sim runner

8. **`sim/topology.rs`**: `TopoConfig` — reuse parsing logic from Phase 1 step 4.
9. **`sim/env.rs`**: env var map from lab state; `${}` interpolation.
10. **`sim/build.rs`**: `build_or_fetch_binary` — git / URL download / path.
    URL download: fetch, detect tar.gz vs bare binary, extract to `<work_dir>/bins/`.
11. **`sim/transfer.rs`**: port `TransferCommand` / `LogReader` from
    `resources/dogfood/`; adapt to run in namespaces via `spawn_in_netns`.
12. **`sim/report.rs`**: `parse_iroh_log(path)` → `TransferResult`;
    `write_results_json(work_dir, results)`.
13. **`sim/runner.rs`**: step executor — all actions; `wait-for` default 300 s;
    `assert` evaluates simple `key == value` / `key != value` expressions.
14. **`src/main.rs`**: `Cli` + wire everything.
15. Write `topos/*.toml` and `sims/*.toml`.
16. End-to-end: `cargo make run-vm -- sims/iroh-1to1.toml`.

### Phase 4 — `count` expansion

17. **`lib.rs` / `topology.rs`**: expand `count = N` into N devices;
    update env var naming (`$NETSIM_IP_fetcher_0`, etc.).
18. **`runner.rs`**: handle `fetchers = [...]` in `kind = "iroh-transfer"`;
    aggregate per-fetcher results in `results.json`.
19. Write `sims/iroh-1toN.toml` family.

---

## 19. Resolved Questions

- **Single-interface shorthand**: always require explicit `[device.name.eth0]`.
  Parser stays uniform; no special-casing.
- **`switch_route` mutability**: `&mut self` throughout.
- **`alloc_global_ipv4`**: inferred from `nat`; no explicit TOML field in v1.
- **`NatMode::None` / `NatMode::Cgnat`**: add both variants.
- **`count`**: moved to Phase 4, not dropped.
- **`wait-for` timeout**: default 300 s; override with `timeout = "600s"`.
- **`PathStats` semantics**: provider end-of-connection signal; not a transfer
  metric; not readiness.  Provider streams until `PathStats`, then gets SIGINT.
- **`connected_via`**: from `ConnectionTypeChanged` events in fetcher log only.
