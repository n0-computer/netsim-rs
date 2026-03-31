# Bidirectional Link Impairment

## Problem

`set_link_condition` applies netem only on the device-side veth (egress). This impairs upload only. Download traffic flows through the unimpaired bridge-side veth. For tests measuring RTT or bidirectional protocols (QUIC, holepunching), the measured impairment is half the expected value.

## Design

Add a `direction` parameter to link condition APIs with three modes:

```rust
pub enum LinkDirection {
    /// Impair egress from device only (current behavior).
    Egress,
    /// Impair ingress to device only (bridge-side veth).
    Ingress,
    /// Impair both directions (default for new API).
    Both,
}
```

**Default is `Both`** — this matches real-world link behavior where latency, loss, and bandwidth are properties of the physical medium. Users who need asymmetric impairment (e.g., modeling bad uplink on good downlink) can use `Egress` or `Ingress` explicitly.

## API Changes

### Device handle

```rust
// Current (unchanged, now applies Both by default):
device.set_link_condition("eth0", LinkCondition::Mobile3G).await?;

// New: explicit direction
device.set_link_condition_dir("eth0", LinkCondition::Mobile3G, LinkDirection::Egress).await?;
```

Or with a builder:
```rust
device.set_link_condition("eth0", LinkCondition::Mobile3G)
    .direction(LinkDirection::Egress)
    .await?;
```

The simple API (`set_link_condition`) uses `Both`. The explicit API lets you choose.

### Sim TOML (runner)

```toml
[[step]]
action = "set-link-condition"
device = "client"
iface = "eth0"
condition = "mobile-3g"
# Optional, defaults to "both":
direction = "egress"  # or "ingress" or "both"
```

### Router downlink

`Router::set_downlink_condition` already impairs the bridge egress to ALL devices. This is conceptually different (router-level, affects everyone) and stays as-is.

## Implementation

### In `qdisc.rs`

`apply_impair_in` currently takes `(netns_mgr, namespace, ifname, impairment)`. No change needed — it already works on any interface in any namespace.

### In `core.rs` (`wire_iface_async`, ~line 2814)

After applying impairment to `dev_ns`/`ifname` (device-side veth), also apply to `gw_ns`/`gw_ifname` (bridge-side veth in router namespace) when direction is `Both` or `Ingress`.

The bridge-side veth name is `v{idx}` where `idx` is the interface index, stored in `DeviceIfaceData`.

### In `handles.rs` (`set_link_condition`)

Accept optional `LinkDirection` parameter. Default to `Both`. Resolve both the device-side and bridge-side veth names, apply netem accordingly.

### In `lab.rs` (presets)

No change to preset values. `LinkCondition::WiFi` etc. define the impairment parameters. The direction is orthogonal.

## Backward Compatibility

**Breaking change**: existing code calling `set_link_condition` will now get bidirectional impairment instead of egress-only. This doubles the effective RTT for latency-based conditions.

Options:
1. **Accept the break** — the old behavior was arguably a bug (users expected symmetric impairment). Bump minor version.
2. **Opt-in**: add `set_link_condition_bidirectional` and keep `set_link_condition` as egress-only. Deprecate later.
3. **Feature flag**: `LinkCondition::Mobile3G.egress_only()` returns a variant that only applies egress.

**Recommendation**: Option 1. The current behavior surprises users. The fix aligns with expectations. The semver check already flags a breaking change for `LabEventKind::TestCompleted`, so this can ride along in a minor bump (pre-1.0).

## Presets Consideration

Current presets define one set of values. For bidirectional application, the same values are applied to both directions. This means:
- `WiFi` with 5ms latency → 5ms each way → 10ms RTT
- `Satellite` with 300ms latency → 300ms each way → 600ms RTT

This is correct for modeling "link has 5ms latency" (5ms each way = 10ms RTT). If users want "10ms RTT total" they should set 5ms latency.

To support asymmetric presets later, `LinkLimits` could gain `ingress_*` fields. Not needed now.

## What NOT to change

- `Router::set_downlink_condition` — different concept (router-wide)
- IFB/ingress redirect — complex, not needed when we can use the bridge-side veth
- Preset values — they define per-direction impairment, applied to both directions
