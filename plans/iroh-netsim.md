# Plan: Iroh Netsims (v3)

Goal: `cargo run -- sims/iroh-1to1.toml` builds a network, runs an iroh transfer
sim, applies scheduled network events, and reports results.

Reference: `resources/chuck/netsim/` — the Python/mininet implementation we're
porting. `resources/dogfood/` — Rust transfer logic to port.

---

## Status: DRAFT v3

Merged from `iroh-netsim v2` + `dynamic-network v2`.

**Key changes from v2:**
- `[[isp]]` / `[[dc]]` / `[[lan]]` removed → unified `[[router]]`
- `add_isp` / `add_dc` / `add_home` / `Gateway` enum removed from Rust API
- Device format changed: per-interface `[device.<name>.<ifname>]` tables
- `DeviceBuilder` replaces the old `add_device(name, Gateway, impair)` signature
- Dynamic-network ops (`set_impair`, `link_down/up`, `switch_route`) merged in
- Env vars now per-interface: `$NETSIM_IP_<device>` and `$NETSIM_IP_<device>_<ifname>`

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

**`alloc_global_ipv4`:**
- `true` → downstream draws from public CIDR pool (`DownstreamPool::Public`)
- `false` → downstream draws from private CIDR pool (`DownstreamPool::Private`)
- Inferred automatically; only override when the default is wrong.

### Rust API

```rust
impl Lab {
    pub fn add_router(
        &mut self,
        name: &str,
        region: Option<&str>,
        upstream: Option<NodeId>,
        nat: Option<NatMode>,
    ) -> Result<NodeId>;
}
```

`NatMode` gains the `Cgnat` variant:

```rust
pub enum NatMode {
    None,
    Cgnat,
    DestinationIndependent,
    DestinationDependent,
}
```

`add_router` infers `alloc_global_ipv4` and `cgnat` from `nat`. If callers need
to override the pool, add `alloc_global_ipv4: bool` as an optional builder step
later; not needed for v1.

---

## 2. Device Format — Per-Interface Tables

Each interface is declared as a sub-table `[device.<name>.<ifname>]`.
Device-level settings go in `[device.<name>]` (optional; may be omitted if
there are no device-level overrides).

```toml
# Simple: single interface
[device.provider.eth0]
gateway = "dc-eu"

# Multi-interface: two pre-wired uplinks
[device.fetcher]
default_via = "eth1"          # eth1 is active at startup; eth0 exists but inactive

[device.fetcher.eth0]
gateway = "isp-eu"
impair  = "mobile"

[device.fetcher.eth1]
gateway = "lan-eu"
impair  = "wifi"
```

### Field semantics

`[device.<name>]` (device-level, optional):

| Field        | Values         | Default                     |
|--------------|----------------|-----------------------------|
| `default_via`| interface name | first interface encountered |

`[device.<name>.<ifname>]` (per-interface, required per interface):

| Field     | Values                                          | Default  |
|-----------|-------------------------------------------------|----------|
| `gateway` | router name                                     | required |
| `impair`  | `"wifi"` / `"mobile"` / `{ rate, loss, latency }` | none  |

### TOML parsing note

The `device` section is parsed as `HashMap<String, toml::Value>` (raw), then
post-processed.  For each device value:
- String-valued keys (`default_via`) are device-level config.
- Table-valued keys (e.g. `eth0`, `eth1`) are treated as interface definitions.

This cleanly separates device metadata from interface entries without
`#[serde(flatten)]` hacks.

---

## 3. Environment Variables

Every process started in a step receives:

| Variable                        | Value                                           |
|---------------------------------|-------------------------------------------------|
| `NETSIM_IP_<device>`            | IP of the `default_via` interface               |
| `NETSIM_IP_<device>_<ifname>`   | IP of the named interface                       |
| `NETSIM_NS_<device>`            | netns name (one namespace per device always)    |

Examples:
- `NETSIM_IP_provider` → provider's `eth0` IP (only interface, hence default)
- `NETSIM_IP_fetcher` → fetcher's `eth1` IP (`default_via = "eth1"`)
- `NETSIM_IP_fetcher_eth0` → fetcher's `eth0` (mobile) IP
- `NETSIM_IP_fetcher_eth1` → fetcher's `eth1` (wifi) IP

Variable name normalisation: device and interface names have `-` replaced with `_`
and are uppercased.

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
}
```

The old `Device` fields `uplink: Option<SwitchId>`, `ip: Option<Ipv4Addr>`,
`impair_upstream: Option<Impair>` are removed.

---

## 5. Rust API — DeviceBuilder

```rust
// In lib.rs

impl Lab {
    /// Start building a device.  Call `.iface(…)` one or more times, then `.build()`.
    pub fn add_device(&mut self, name: &str) -> DeviceBuilder<'_>;
}

pub struct DeviceBuilder<'a> {
    lab:         &'a mut Lab,
    name:        String,
    interfaces:  Vec<IfaceCfg>,   // in push order
    default_via: Option<String>,
}

struct IfaceCfg {
    ifname:  String,
    gateway: NodeId,
    impair:  Option<Impair>,
}

impl<'a> DeviceBuilder<'a> {
    /// Add an interface.  `gateway` is any `NodeId` returned by `add_router`.
    pub fn iface(mut self, ifname: &str, gateway: NodeId, impair: Option<Impair>) -> Self;

    /// Override which interface carries the default route (default: first added).
    pub fn default_via(mut self, ifname: &str) -> Self;

    /// Finalise the device; returns its `NodeId`.
    pub fn build(self) -> Result<NodeId>;
}
```

Usage:

```rust
// Simple (single interface)
let provider = lab.add_device("provider")
    .iface("eth0", dc_eu, None)
    .build()?;

// Multi-interface (pre-wired)
let fetcher = lab.add_device("fetcher")
    .iface("eth0", isp_eu, Some(Impair::Mobile))
    .iface("eth1", lan_eu, Some(Impair::Wifi))
    .default_via("eth1")
    .build()?;
```

`add_isp`, `add_dc`, `add_home`, and the `Gateway` enum are **removed**.
All tests are updated to use `add_router` + `DeviceBuilder`.

---

## 6. Build Changes — Multi-Interface Wiring

`DevBuild` becomes per-interface (`IfaceBuild`).  `wire_device` becomes
`wire_iface`:

```rust
struct IfaceBuild {
    dev_ns:     String,
    gw_ns:      String,
    gw_ip:      Ipv4Addr,      // gateway-side bridge IP (default route for device)
    gw_br:      String,
    dev_ip:     Ipv4Addr,
    prefix_len: u8,
    impair:     Option<Impair>,
    ifname:     String,        // "eth0", "eth1", …
    is_default: bool,          // only this interface gets `ip route add default`
    idx:        u64,           // globally unique; drives veth naming
}
```

`wire_iface` is identical to the current `wire_device` except:
- The interface is renamed to `dev.ifname` instead of always `"eth0"`.
- `add_default_route_v4` is only called when `is_default == true`.

`LabCore::build` collects one `IfaceBuild` per `(device, interface)` pair and
calls `wire_iface` for each.

---

## 7. Dynamic Network Operations

### 7a. `qdisc::remove_qdisc` (returns `Result`)

The existing `remove_qdisc(ns, ifname)` returns `()` and silently swallows
errors.  Add a fallible version used by `set_impair` / `switch_route`:

```rust
pub(crate) fn remove_qdisc_r(ns: &str, ifname: &str) -> Result<()> {
    let status = run_in_netns(ns, {
        let mut cmd = Command::new("tc");
        cmd.args(["qdisc", "del", "dev", ifname, "root"]);
        cmd.stderr(std::process::Stdio::null());
        cmd
    })?;
    // exit code 2 = no such qdisc — acceptable
    if !status.success() && status.code() != Some(2) {
        bail!("tc qdisc del failed on {} in {}", ifname, ns);
    }
    Ok(())
}
```

### 7b. `Lab::set_impair`

```rust
impl Lab {
    /// Replace (or remove) the tc impairment on a device's named interface.
    /// Omitting `ifname` targets the `default_via` interface.
    pub fn set_impair(
        &self,
        device: &str,
        ifname: Option<&str>,
        impair: Option<Impair>,
    ) -> Result<()>;
}
```

Implementation:
1. Resolve device → look up `ifname` (or `default_via`).
2. `Some(impair)` → `qdisc::apply_impair(ns, ifname, limits)`.
3. `None` → `qdisc::remove_qdisc_r(ns, ifname)`.

### 7c. `Lab::link_down` / `Lab::link_up`

```rust
impl Lab {
    pub fn link_down(&self, device: &str, ifname: &str) -> Result<()>;
    pub fn link_up(&self,   device: &str, ifname: &str) -> Result<()>;
}
```

Implementation: run `ip link set <ifname> down/up` inside the device namespace
via `with_netns_thread`.

### 7d. `Lab::switch_route`

```rust
impl Lab {
    /// Switch a device's active default route to the named interface.
    /// Re-applies the impairment configured for that interface.
    /// Updates the device's tracked `default_via`.
    pub fn switch_route(&self, device: &str, to: &str) -> Result<()>;
}
```

`to` is always an interface name (`"eth0"`, `"eth1"`).  No `"primary"` /
`"secondary"` aliases — use explicit names.

Implementation:

```rust
pub fn switch_route(&self, device: &str, to: &str) -> Result<()> {
    let (ns, gw_ip, ifname, impair) = {
        let dev = self.resolve_device(device)?;
        let iface = dev.iface(to)
            .ok_or_else(|| anyhow!("unknown interface '{}' on device '{}'", to, device))?;
        let gw_ip = self.core.router_downlink_gw_for_switch(iface.uplink)?;
        (dev.ns.clone(), gw_ip, iface.ifname.clone(), iface.impair)
    };

    with_netns_thread(&ns, move || {
        Command::new("ip").args(["route", "del", "default"]).status()?;
        Command::new("ip")
            .args(["route", "add", "default", "via", &gw_ip.to_string(), "dev", &ifname])
            .status()?;
        Ok(())
    })?;

    match impair {
        Some(imp) => apply_impair_in(&ns, &ifname, imp),
        None      => { qdisc::remove_qdisc_r(&ns, &ifname).ok(); }
    }

    // Persist the new active path
    self.core.set_device_default_via(device, to)?;
    Ok(())
}
```

`LabCore` needs:
```rust
pub fn router_downlink_gw_for_switch(&self, sw: SwitchId) -> Result<Ipv4Addr>;
pub fn set_device_default_via(&mut self, name: &str, ifname: &str) -> Result<()>;
```

Note: `switch_route` takes `&self` but mutates `default_via`.  Use
`RefCell<HashMap<…>>` or change to `&mut self`.  **Prefer `&mut self`** — callers
that need `switch_route` during a sim step already have exclusive access.

---

## 8. Sim File Format

```toml
[sim]
name     = "iroh-wifi-to-mobile"
topology = "wifi-to-mobile"     # loads topos/wifi-to-mobile.toml

[binary]
repo    = "https://github.com/n0-computer/iroh"
commit  = "main"
example = "transfer"
```

Inline topology (no `topology` ref — topology tables live in the same file):

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

When `topology` is set the top-level router/device tables are loaded from
`topos/<topology>.toml` and must not appear in the sim file itself.

---

## 9. Step Actions

| `action`       | Required fields                                | Optional                                  |
|----------------|------------------------------------------------|-------------------------------------------|
| `spawn`        | `id`, `device`, `cmd` **or** `kind`            | `ready_when`, `ready_after`, `captures`   |
| `run`          | `device`, `cmd`                                | —                                         |
| `wait`         | `duration`                                     | —                                         |
| `wait-for`     | `id`                                           | `timeout`                                 |
| `set-impair`   | `device`, `impair`                             | `interface` (default: `default_via`)      |
| `switch-route` | `device`, `to`                                 | —                                         |
| `link-down`    | `device`, `interface`                          | —                                         |
| `link-up`      | `device`, `interface`                          | —                                         |
| `assert`       | `check`                                        | `timeout`                                 |

`to` in `switch-route` is an interface name string, e.g. `"eth0"`.

```toml
[[step]]
action = "set-impair"
device = "fetcher"
impair = "mobile"           # targets default_via; preset or manual table

[[step]]
action    = "set-impair"
device    = "fetcher"
interface = "eth0"          # explicit interface
impair    = { loss = 2.0, latency = 5000, rate_kbit = 1000 }

[[step]]
action = "set-impair"
device = "fetcher"
impair = "none"             # remove impairment

[[step]]
action = "switch-route"
device = "fetcher"
to     = "eth0"             # switch from wifi (eth1) to mobile (eth0)

[[step]]
action    = "link-down"
device    = "fetcher"
interface = "eth1"

[[step]]
action    = "link-up"
device    = "fetcher"
interface = "eth1"
```

### `spawn` with `kind = "iroh-transfer"`

```toml
[[step]]
action    = "spawn"
kind      = "iroh-transfer"
id        = "xfer"
provider  = "provider"
fetcher   = "fetcher"           # or fetchers = ["f-0", "f-1"]
relay_url = "http://..."        # optional
```

Built-in handler (ported from `resources/dogfood/`):
- Starts provider in its netns; reads JSON stdout until `EndpointBound`.
- Starts fetcher(s) with captured `endpoint_id`.
- Reads `PathStats` → captures `connected_via` ("direct" / "relay").
- Exposes `xfer.endpoint_id`, `xfer.connected_via` for `assert` steps.

### Variable substitution in `cmd`

- `$NETSIM_IP_<device>` — default-via IP
- `$NETSIM_IP_<device>_<ifname>` — specific interface IP
- `$NETSIM_NS_<device>` — netns name
- `${binary}` — path to built binary
- `${data}` — sim-specific data directory
- `${<id>.<capture>}` — value captured from a prior `spawn`

---

## 10. Binary Spec

```toml
[binary]
repo    = "https://github.com/n0-computer/iroh"
commit  = "main"          # branch, tag, or SHA
example = "transfer"      # cargo --example <name>
# OR:
path    = "/path/to/prebuilt"
```

Build function (`src/sim/build.rs`):
- Clone if no `.git`; fetch + checkout `commit`; `cargo build --example … --release`.
- Skip build if binary mtime > source mtime.

---

## 11. Module Layout

```
src/
  lib.rs          — Lab, DeviceBuilder, add_router, TOML config parsing
  core.rs         — LabCore, Device/DeviceIface, Netlink, build
  qdisc.rs        — tc helpers; add remove_qdisc_r
  sim/
    mod.rs        — SimConfig, run_sim
    topology.rs   — TopoConfig (shared between standalone topos/ and inline)
    build.rs      — build_iroh_binary
    transfer.rs   — iroh-transfer kind (port from resources/dogfood/)
    runner.rs     — step executor
    env.rs        — env-var injection + ${} interpolation
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

## 12. CLI

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

`--set` applies dotted-key overrides to any scalar in the sim config,
e.g. `--set binary.commit=abc123`.

---

## 13. Cargo.toml additions

```toml
[dependencies]
clap       = { version = "4", features = ["derive"] }
serde_json = "1"
tokio      = { version = "1", features = ["full"] }   # replace partial feature list
regex      = "1"
# iroh = "0.96"   # add when implementing the transfer kind
```

---

## 14. Implementation Order

Phase 1 — core types + builder (keep all existing tests green throughout):

1. **`core.rs`**: replace `Device` with multi-iface version (`interfaces: Vec<DeviceIface>`, `default_via: String`); add `DeviceIface`; add `router_downlink_gw_for_switch`, `set_device_default_via`.
2. **`lib.rs`**: add `DeviceBuilder`; remove `add_isp`/`add_dc`/`add_home`/`Gateway` enum; add `add_router`; update all tests to use the new API.
3. **`core.rs` build**: `DevBuild` → `IfaceBuild`; `wire_device` → `wire_iface`; loop over all interfaces per device; only add default route for `default_via`.
4. **`lib.rs` TOML config**: parse `[[router]]` (unified) + `[device.name.ethN]` tables; remove old `[[isp]]`/`[[dc]]`/`[[lan]]` parsing; update `load_from_toml` test.

Phase 2 — dynamic ops:

5. **`qdisc.rs`**: add `remove_qdisc_r`.
6. **`lib.rs`**: `Lab::set_impair`, `Lab::link_down`, `Lab::link_up`, `Lab::switch_route`.
7. **Tests**: add tests for `set_impair` (RTT changes), `link_down/up` (connectivity lost/restored), `switch_route` (RTT changes after switching).

Phase 3 — sim runner:

8. **`src/sim/topology.rs`**: `TopoConfig` — parse `[[router]]` + `[device…]` sections (reuse logic from `lib.rs` step 4).
9. **`src/sim/env.rs`**: build env var map from lab state; `${}` interpolation.
10. **`src/sim/build.rs`**: `build_iroh_binary` (git clone + cargo build).
11. **`src/sim/transfer.rs`**: `TransferCommand` / `LogReader` ported from `resources/dogfood/`.
12. **`src/sim/runner.rs`**: step executor — `spawn`, `run`, `wait`, `wait-for`, `set-impair`, `switch-route`, `link-down`, `link-up`, `assert`.
13. **`src/main.rs`**: `Cli` + wire everything together.
14. Write `topos/*.toml` and `sims/*.toml`.
15. End-to-end: `cargo make run-vm` with `sims/iroh-1to1.toml`.

---

## 15. Open Questions (to resolve before starting implementation)

**Q1 — Single-interface shorthand**
For the common case of a device with one interface, is `[device.provider.eth0]`
always required, or should we also allow:

```toml
[device.provider]
gateway = "dc-eu"
```

…treating a `gateway` key directly in `[device.provider]` as an implicit `eth0`?

Tradeoff: shorthand reduces line noise for 80 % of devices, but adds a parsing
special-case.  Recommendation: **require explicit `eth0`** for simplicity; the
parser stays uniform and users can always grep for `gateway`.

**Q2 — `switch_route` mutability**
`switch_route` needs to update `device.default_via`, but `Lab`'s public API
uses `&self` for query methods.  Options:

- Change `switch_route` (and `set_impair`) to `&mut self` — callers need `mut lab`.
- Wrap `devices` in `RwLock<…>` — more complex, enables `&self` everywhere.

Recommendation: **`&mut self`** — the runner already has exclusive `&mut Lab`.

**Q3 — `count` device shorthand**
The v2 plan had `count = 5` on a device to generate `fetcher-0..fetcher-4`.
Keep or drop for now?

Recommendation: **drop** — not needed for any immediate sim; add later.

**Q4 — `NatMode::None` in Rust**
Currently `NatMode` has only `DestinationIndependent` and `DestinationDependent`.
We need `NatMode::None` and `NatMode::Cgnat`.  This is straightforward, but
mention it explicitly for the implementer.

**Q5 — `alloc_global_ipv4` override**
The `[[router]]` spec mentions `alloc_global_ipv4` as an override field.  Is it
ever needed for the immediate sims, or can it always be inferred from `nat`?

Recommendation: **infer always** for v1; drop the explicit field.  Add it back
only if a sim requires it.
