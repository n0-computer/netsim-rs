# Project History

Chronological record of significant changes to patchbay. Moved from AGENTS.md to
keep agent-facing instructions concise.

For current architecture and conventions, see [AGENTS.md](AGENTS.md).

---

## Recent Changes (newest first)

### No-Panics Refactor
- Device/Router handles return `Result` or `Option` instead of panicking on removed nodes.
- `spawn()` returns `Result<JoinHandle>`.
- `with_device`/`with_router` return `Option<R>`.

### Mutex/Lock Architecture Overhaul
- `LabInner` struct with `netns` and `cancel` outside the topology mutex.
- `with()`/`with_mut()` helpers on handles for lock-access boilerplate.
- Cached `name`/`ns` on Device/Router/Ix (zero-lock for common accessors).
- Per-node `tokio::sync::Mutex<()>` for operation serialization.
- `parking_lot::Mutex` for the topology lock (no poisoning, compile-time await guard).
- All handle mutation methods made async.
- Pre-await reads combined into single lock acquisitions.

### Phase 2: Enhanced Link Conditions
- Expanded `LinkLimits` with `jitter_ms`, `reorder_pct`, `duplicate_pct`, `corrupt_pct`.
- Added presets: `Lan`, `WifiBad`, `Mobile4G`, `Mobile3G`, `Satellite`, `SatelliteGeo`.
- `LinkCondition::Manual` now wraps `LinkLimits` (was inline struct fields).
- Old `Mobile` preset removed; TOML `"mobile"` deserializes to `Mobile4G`.
- `LinkCondition::to_limits()` converts any preset to concrete `LinkLimits`.
- `RouterBuilder::downlink_condition(LinkCondition)` applies impairment at build time.
- `tc netem` command now conditionally emits jitter/reorder/duplicate/corrupt args.

### Phase 1a: NatConfig Builder API
- Added `NatConfig`, `NatConfigBuilder`, `ConntrackTimeouts` structs.
- `Nat::to_config()` expands presets into `NatConfig`.
- `generate_nat_rules()` builds nftables from `NatConfig` (mapping/filtering enums).

### Phase 1: NAT Presets + API Rename
- Added `Nat` enum: `None`, `Home`, `Corporate`, `Cgnat`, `CloudNat`, `FullCone`.
- Implemented fullcone dynamic nftables map for reliable EIM.
- APDF filtering via `ct state established,related` in forward filter chain.
- Home NAT hole-punching verified and tested.
- API renames: `switch_route` -> `set_default_route`, `switch_uplink` -> `replug_iface`,
  `rebind_nats` -> `flush_nat_state`, `set_impair` -> `set_link_condition`,
  `impair_link` -> `set_link_condition`, `impair_downlink` -> `set_downlink_condition`.

### IPv6 Dual-Stack + V6-Only
- `IpSupport` enum: `V4Only`, `DualStack`, `V6Only`.
- DAD consolidation, ULA addressing for downstream, all tests pass.

### New Lab API
- `Lab` with `Arc<LabInner>`, `Device`/`Router` handles.
- Builder API: `lab.add_router("name").nat(Nat::Home).build().await?`.
- Instant construction - topology built on `build()`, not `Lab::load()`.

### Rootless User-Namespace Bootstrap
- ELF constructor bootstrap enters unprivileged user namespace before Tokio starts.
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
