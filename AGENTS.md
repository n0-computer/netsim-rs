# AGENTS.md

Project: **netsim-rs** — Linux network-namespace lab for NAT/routing/impairment experiments.

This file is the single entry point for agents. Read it fully before working. Follow all rules below — they are mandatory, not suggestions.

## Key Resources (read as needed)

| File | Purpose |
|------|---------|
| [`AGENTS.md`](AGENTS.md) | **You are here.** Architecture, conventions, mandatory workflow. |
| [`plans/PLAN.md`](plans/PLAN.md) | Plan index — in-progress, open, partial, completed plans. |
| [`plans/real-world-conditions.md`](plans/real-world-conditions.md) | Current active plan: NAT, impairment, regions, firewall, API. |
| [`REVIEW.md`](REVIEW.md) | Open and completed review items. |
| [`HOLEPUNCH.md`](HOLEPUNCH.md) | NAT implementation findings — fullcone maps, APDF filtering, nftables lessons. |
| [`HISTORY.md`](HISTORY.md) | Chronological changelog (moved from old AGENTS.md). |
| [`docs/`](docs/) | Additional documentation (network patterns guide, etc). |

---

## Architecture

### Crate: `netsim-core`

The library crate. All network simulation logic lives here.

- **`src/core.rs`** — `NetworkCore`: topology state, router/device/switch records, NAT rule generation, nftables helpers.
- **`src/lab.rs`** — Public API: `Lab`, `Device`, `Router`, `Ix` handles; builders (`RouterBuilder`, `DeviceBuilder`); types (`Nat`, `NatConfig`, `Impair`, `ImpairLimits`, `IpSupport`, etc).
- **`src/netns.rs`** — `NetnsManager`: two workers per namespace (async tokio + sync thread). All `setns(2)` happens here.
- **`src/qdisc.rs`** — All `tc` command invocation: netem (latency/jitter/loss/reorder/duplicate/corrupt), TBF (rate), HTB (region latency).
- **`src/netlink.rs`** — `Netlink` struct wrapping `rtnetlink::Handle` for link/addr/route operations.
- **`src/test_utils.rs`** — UDP reflector/probe helpers for integration tests.
- **`src/tests.rs`** — Integration test suite (~90 tests).
- **`src/config.rs`** — TOML config structures for `Lab::load`.
- **`src/userns.rs`** — ELF ctor bootstrap into unprivileged user namespace.
- **`src/lib.rs`** — Re-exports, `check_caps()`.

### Key Design Rules

1. **Never block tokio thread with TCP/UDP I/O.** Use `spawn_task_in_netns` + `tokio::net` + `tokio::time::timeout`.
2. **Sync `run_closure_in` is for fast non-I/O work only** (sysctl, `Command::spawn`).
3. **NAT rules are generated from `NatConfig`**, not from `Nat` variants directly. `Nat::to_config()` expands presets.
4. **Tests use `#[tokio::test(flavor = "current_thread")]`** due to `setns` thread-local behavior.

### Workspace

```
netsim-core/    — Library crate (main development target)
netsim-utils/   — CLI utilities
netsim/         — Binary crate (sim runner, inspect)
netsim-vm/      — VM orchestration
ui/             — Vite + React browser UI
```

### Permissions

No root required. The process bootstraps into an unprivileged user namespace via ELF ctor before Tokio starts. Effective UID becomes 0 inside the user namespace.

### Naming / Prefixes

- Namespaces: `lab<N>-r<id>` (router), `lab<N>-d<id>` (device), `lab<N>-root`
- Bridges: `br-p<pid><n>-<sw_id>`
- Veths: `lab-p<pid><n>e<id>` / `lab-p<pid><n>g<id>`

---

## Mandatory Workflow

### Before every commit

Run these in order. All must pass with zero warnings/errors:

ALWAYS add a timeout to test runs, like 90s.

```bash
cargo fmt
cargo clippy -p netsim-core --tests --fix --allow-dirty
cargo check -p netsim-core --tests
cargo nextest run -p netsim-core             # use nextest, not cargo test; parallelism in .config/nextest.toml
```

when that is clean, run cargo check for the full workspace and test the other crates individually

If the UI was modified, also run: `cd ui && npm run test:e2e`

### Commit conventions

- Format: `feat: short description`, `fix: ...`, `refactor: ...`, `test: ...`, `docs: ...`, `chore: ...`
- Include meaningful body for non-trivial changes.
- Do not commit without being asked. Stage files, then ask.

### Code quality

- **Document all public items.** Follow official Rust doc conventions.
- **No warnings.** Treat clippy and rustc warnings as errors.
- **Test coverage.** Add tests for new functionality. All presets, all code paths.
- **No over-engineering.** Only add what's needed for the current task.

---

## Plans

Plans live in `plans/`. The index is [`plans/PLAN.md`](plans/PLAN.md) with sections:

1. `# In progress` — currently being worked on
2. `# Open` — ready to start; listed with priority (1–5, default 2)
3. `# Partial` — mostly done, one-line note on what remains
4. `# Completed` — done; listed with priority

Omit empty sections.

Each plan file **must start with a `## TODO` checklist**:
- First item: `- [x] Write plan` (always checked)
- Middle items: implementation steps (`[x]` done, `[ ]` pending)
- Last item: `- [ ] Final review`

### Review commands

- **`review`** — find completed plans with unchecked `Final review`, review implementation, check it off.
- **`review general`** — scan codebase for quality issues, update `REVIEW.md`.

### REVIEW.md format

- `# Open` — unresolved issues with full details.
- `# Completed` — resolved issues, one-liner per item with ✅.

---

## NAT Implementation (Summary)

Full details in [`HOLEPUNCH.md`](HOLEPUNCH.md).

| Preset | Mapping | Filtering | nftables approach |
|--------|---------|-----------|-------------------|
| `Nat::Home` | EIM | APDF | fullcone map + `snat to <ip>` + forward filter |
| `Nat::FullCone` | EIM | EIF | fullcone map + `snat to <ip>` |
| `Nat::Corporate` | EDM | APDF | `masquerade random` |
| `Nat::CloudNat` | EDM | APDF | `masquerade random` (longer timeouts) |
| `Nat::Cgnat` | — | — | plain `masquerade` on IX iface |

Key finding: `snat to <ip>` does NOT preserve ports reliably. The fullcone dynamic map is required for EIM.

---

## Impairment Presets (Summary)

| Preset | Latency | Jitter | Loss | Rate |
|--------|---------|--------|------|------|
| `Lan` | 0 | 0 | 0% | — |
| `Wifi` | 5ms | 2ms | 0.1% | — |
| `WifiBad` | 40ms | 15ms | 2% | 20 Mbit |
| `Mobile4G` | 25ms | 8ms | 0.5% | — |
| `Mobile3G` | 100ms | 30ms | 2% | 2 Mbit |
| `Satellite` | 40ms | 7ms | 1% | — |
| `SatelliteGeo` | 300ms | 20ms | 0.5% | 25 Mbit |

---

## Common Pitfalls

- **Host root leakage**: never run lab operations in host root netns.
- **TC warnings**: use `r2q 1000` in HTB root to avoid quantum warnings.
- **TC stderr suppressed**: all `tc` commands suppress stderr (known tech debt, see REVIEW.md).
- **Port base in tests**: each test combo needs unique ports to avoid conntrack collisions.
- **Holepunch timing**: after receiving a probe, send extra "ack" packets before returning (APDF timing asymmetry).
