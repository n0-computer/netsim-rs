# Project History

Chronological record of significant changes to patchbay. Moved from AGENTS.md to
keep agent-facing instructions concise.

For current architecture and conventions, see [AGENTS.md](AGENTS.md).

---

## Recent Changes (newest first)

### Phase 2: Enhanced Impairment
- Expanded `ImpairLimits` with `jitter_ms`, `reorder_pct`, `duplicate_pct`, `corrupt_pct`.
- Added presets: `Lan`, `WifiBad`, `Mobile4G`, `Mobile3G`, `Satellite`, `SatelliteGeo`.
- `Impair::Manual` now wraps `ImpairLimits` (was inline struct fields).
- Old `Mobile` preset removed; TOML `"mobile"` deserializes to `Mobile4G`.
- `Impair::to_limits()` converts any preset to concrete `ImpairLimits`.
- `RouterBuilder::downlink_condition(Impair)` applies impairment at build time.
- `tc netem` command now conditionally emits jitter/reorder/duplicate/corrupt args.

### Phase 1a: NatConfig Builder API
- Added `NatConfig`, `NatConfigBuilder`, `ConntrackTimeouts` structs.
- `Nat::to_config()` expands presets into `NatConfig`.
- `generate_nat_rules()` builds nftables from `NatConfig` (mapping/filtering enums).
- `RouterBuilder::nat_config()` and `Router::set_nat_config()` for custom NAT.

### Phase 1: NAT Presets + API Rename
- Added `Nat` enum: `None`, `Home`, `Corporate`, `Cgnat`, `CloudNat`, `FullCone`.
- Implemented fullcone dynamic nftables map for reliable EIM.
- APDF filtering via `ct state established,related` in forward filter chain.
- Home NAT hole-punching verified and tested.
- API renames: `switch_route` → `set_default_route`, `switch_uplink` → `replug_iface`,
  `rebind_nats` → `flush_nat_state`, `set_impair` → `set_link_condition`,
  `impair_link` → `set_link_condition`, `impair_downlink` → `set_downlink_condition`.
- Old names kept as `#[deprecated]` aliases.

### IPv6 Dual-Stack + V6-Only
- `IpSupport` enum: `V4Only`, `DualStack`, `V6Only`.
- DAD consolidation, ULA addressing for downstream, all tests pass.

### New Lab API
- `Lab` with `Arc<Mutex<NetworkCore>>`, `Device`/`Router` handles.
- Builder API: `lab.add_router("name").nat(Nat::Home).build().await?`.
- Instant construction — topology built on `build()`, not `Lab::load()`.

### Rootless User-Namespace Bootstrap
- ELF ctor bootstrap enters unprivileged user namespace before Tokio starts.
- No root or file capabilities required.

### Earlier History
- Config-driven sim flow, iroh integration layout.
- Relay/QAD runtime wiring, transfer steps.
- FD-only netns backend, namespace lifecycle via in-process FD registry.
- VM orchestration (patchbay-vm), QEMU artifact staging.
- Browser UI (Vite + React), live progress, log viewer.
- Sim runner with progress.json, manifest.json, combined reports.
- Netlink-based cleanup, prefix isolation, Ctrl-C handling.
- NAT test harness + matrix coverage.
- NetnsManager with worker threads + single-thread Tokio per namespace.
