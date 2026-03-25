# v1: Unified CLI, Compare, Metrics

## Overview

Consolidate `patchbay-runner` and `patchbay-vm` into a single `patchbay` CLI with
feature-gated backends, add `compare` mode for regression testing across git refs,
and add lightweight per-device metrics recording via tracing.

All work paths consolidate under `.patchbay/`. Image cache stays in
`~/.local/share/patchbay/` (shared across projects).

---

## CLI Structure

```
patchbay
├── test [filter]                         # run cargo test (native or VM)
│   [--ignored]                           # include ignored tests
│   [--ignored-only]                      # run only ignored tests
│   [-p pkg] [--test name]               # cargo test selectors
│   [--vm [qemu|container]]              # force VM backend
│   [-- extra-cargo-args]
│
├── run <sims...>                         # run simulations
│   [--vm [qemu|container]]
│   [--verbose] [--open] [--timeout T]
│
├── compare
│   ├── test [filter] <ref> [ref2]        # compare test results
│   │   [--ignored] [--ignored-only]
│   │   [-p pkg] [--test name]
│   │
│   └── run <sims...> <ref> [ref2]        # compare sim results
│
├── serve [dir]                           # serve UI
│   [--bind addr] [--open] [--testdir]
│
├── vm                                    # direct VM control
│   ├── up [--recreate] [--backend qemu|container]
│   ├── down
│   ├── status
│   ├── ssh [cmd...]
│   └── cleanup
│
├── inspect <topo.toml>                   # interactive ns debugging
└── run-in <node> <cmd...>               # exec in inspect ns
```

### Backend auto-detection

```
Linux           → native (patchbay crate, namespaces)
macOS + ARM     → check `container` CLI exists → container, else qemu
macOS + x86     → qemu
other           → qemu
```

Override: `--vm` (no value = auto-detect VM), `--vm qemu`, `--vm container`.

### Compare semantics

- One ref: `patchbay compare test main` → worktree vs `main`
- Two refs: `patchbay compare test main abc123` → `main` vs `abc123` (no worktree)
- Creates git worktrees in `.patchbay/tree/{ref}/`
- Runs the full test/sim suite in each worktree (sequential)
- Writes compare manifest as a batch under `.patchbay/work/compare-{timestamp}/`
- Compare is itself a batch, so it shows up in UI as a batch with left/right runs
- Prints quick pass/fail + time summary and score
- Worktrees removed if unchanged (git diff empty), kept otherwise

### Test delegation

All `patchbay test` and `patchbay compare test` commands add
`RUSTFLAGS="--cfg patchbay_test"` to cargo invocations (enables conditional
compilation in test code, e.g., `#[cfg(patchbay_test)]`).

On native: prefer `cargo nextest run` if installed, else `cargo test` (warn once
that nextest gives better structured output). Forward filter, `--ignored`,
`--ignored-only`, package/test selectors, and extra args.

On VM: cross-compile to musl, stage binaries, run in guest (existing flow).

**testdir integration:** The `testdir` crate writes to `target/testdir-current/`
and has no env var override. After tests complete, copy `testdir-current/` into
`.patchbay/work/{run}/testdir/` so results are co-located with other run artifacts
and visible in the UI.

---

## Consolidated Paths

```
.patchbay/
├── work/              # run output (was .patchbay-work)
│   ├── latest -> ...
│   ├── sim-YYMMDD-HHMMSS/
│   └── compare-YYMMDD-HHMMSS/   # compare is a batch
│       ├── left-{ref}/           # run results for ref1
│       ├── right-{ref}/          # run results for ref2
│       └── summary.json          # compare manifest
├── vm/                # VM state (was .qemu-vm)
├── tree/              # git worktrees for compare
│   ├── main/
│   └── abc123/
└── cache/             # binary cache (project-local)
    └── binaries/

~/.local/share/patchbay/
└── images/            # shared VM base images (unchanged)
```

`.patchbay/` should be gitignored.

Path migration: check for old `.patchbay-work` and `.qemu-vm`, print one-line
warning pointing to new location, do NOT auto-migrate.

---

## Commit Strategy

### Commit 0: Rename "invocation" → "batch"

**Goal:** Rename everywhere: Rust types, API endpoints, UI code, CSS.
Separate commit because it touches many files but is a trivial rename.

Changes:
- Rust: `invocation` → `batch` in structs, fields, API paths
- Server: `/api/invocations/` → `/api/batches/` (keep `/api/invocations/` as
  alias for backward compat since links are shared on Discord)
- UI: rename in types, components, routes. Keep `#/inv/` route as redirect
  to `#/batch/` for backward compat
- UI routing: switch from hash routes to real routes. Server returns index.html
  for all non-`/api/` and non-asset paths (wildcard fallback)

### Commit 1: Pure refactor — extract libraries from CLIs

**Goal:** Make `patchbay-runner` and `patchbay-vm` into libraries, create `patchbay-cli`.
Zero behavior change.

Changes:
- `patchbay-runner/src/main.rs` → move CLI parsing + dispatch into `patchbay-cli`
  - Keep `sim/` module as library (`pub mod sim` in `lib.rs`)
  - Keep `init.rs` (userns bootstrap)
  - Remove `[[bin]]` from `patchbay-runner/Cargo.toml`, keep as `[lib]`
  - The `inspect`/`run-in` code moves to `patchbay-cli` (it's CLI-only)

- `patchbay-vm/src/main.rs` → move CLI parsing + dispatch into `patchbay-cli`
  - `common.rs`, `qemu.rs`, `container.rs`, `util.rs` stay as library
  - Add `pub` to module-level functions needed by CLI
  - Remove `[[bin]]` from `patchbay-vm/Cargo.toml`, keep as `[lib]`

- New crate: `patchbay-cli/`
  - `Cargo.toml` with feature flags:
    - `native` (default on linux) → depends on `patchbay`, `patchbay-runner`
    - `vm-qemu` → depends on `patchbay-vm`
    - `vm-container` → depends on `patchbay-vm`
    - `serve` (default) → depends on `patchbay-server`
  - `src/main.rs` — unified clap CLI, dispatches to runner/vm libs
  - Binary name: `patchbay`

- Update workspace `Cargo.toml` to add `patchbay-cli`
- Update paths: `.patchbay-work` → `.patchbay/work`, `.qemu-vm` → `.patchbay/vm`

**LOC estimate:** ~300 new in patchbay-cli, ~200 removed from runner+vm mains.
Net small because it's mostly moving code.

### Commit 2: Add `patchbay test` (native + VM)

**Goal:** `patchbay test` delegates to cargo test on native, VM test flow on VM.

Changes:
- `patchbay-cli/src/test.rs` — new module
  - Native path: detect nextest (`which cargo-nextest`), prefer if found, else
    `cargo test` with one-time warning
  - Sets `RUSTFLAGS="--cfg patchbay_test"` on all cargo commands
  - Maps `--ignored` → `-- --ignored`, `--ignored-only` → `-- --ignored`
  - VM path: call into `patchbay_vm::run_tests()` (existing)
  - Parse test output for pass/fail/ignore counts (both cargo test and nextest)
  - After tests: copy `target/testdir-current/` into `.patchbay/work/` run dir
  - Support `--vm` override

**LOC estimate:** ~200 new.

### Commit 3: Metrics recording — `device.record()` + builder + iroh-metrics

**Goal:** Lightweight per-device metrics, stored as JSONL, viewable in UI.

**Format:** `device.<name>.metrics.jsonl`
```jsonl
{"t":1679000000.123,"m":{"throughput_bytes":1234.0,"connections_active":5}}
{"t":1679000000.456,"m":{"latency_ms":42.5}}
```

Each line is a batch of key-value pairs sharing one timestamp. This handles both
single metrics and bulk emission (iroh-metrics, custom structs).

**Tracing approach:** Since tracing field names must be compile-time, we serialize
the metrics map to a JSON string and emit as a single known field:

```rust
tracing::info!(
    target: "patchbay::_metrics",
    metrics_json = %json_string,
);
```

The namespace subscriber (already in `tracing.rs` with `JsonFieldVisitor`)
intercepts this target, parses `metrics_json`, prepends timestamp, writes to
the per-device metrics file.

**Device tracing handle:** Currently `device.run_sync(|| tracing::info!(...))` is
needed to emit to the right file. This is wasteful for metrics. Instead:

```rust
impl Device {
    /// Enter this device's tracing context. Returns a guard; while held,
    /// tracing events are routed to this device's output files.
    pub fn enter_tracing(&self) -> tracing::dispatcher::DefaultGuard {
        let dispatch = self.tracing_dispatch();
        tracing::dispatcher::set_default(&dispatch)
    }

    /// Record a single metric.
    pub fn record(&self, key: &str, value: f64) {
        let _guard = self.enter_tracing();
        let json = format!("{{\"{key}\":{value}}}");
        tracing::info!(target: "patchbay::_metrics", metrics_json = %json);
    }

    /// Record multiple metrics at once.
    pub fn metrics(&self) -> MetricsBuilder<'_> {
        MetricsBuilder { device_name: self.name().to_string(), dispatch: self.tracing_dispatch(), fields: serde_json::Map::new() }
    }
}

/// Builder for batch metric emission.
pub struct MetricsBuilder { ... }
impl MetricsBuilder {
    pub fn field(mut self, key: &str, value: f64) -> Self {
        self.fields.insert(key.to_string(), value.into());
        self
    }
    /// Emit all fields as a single metrics line.
    pub fn emit(self) {
        let _guard = tracing::dispatcher::set_default(&self.dispatch);
        let json = serde_json::to_string(&self.fields).unwrap();
        tracing::info!(target: "patchbay::_metrics", metrics_json = %json);
    }
}
```

**iroh-metrics integration** (optional feature `iroh-metrics`):

```rust
#[cfg(feature = "iroh-metrics")]
impl Device {
    /// Record all metrics from an iroh-metrics MetricsGroup.
    pub fn record_metrics(&self, group: &impl iroh_metrics::MetricsGroup) {
        // MetricsGroup exposes iter() or encode() to get name/value pairs
        // Serialize to JSON map, emit as single metrics line
        let mut builder = self.metrics();
        for (name, value) in group.iter() {
            builder = builder.field(name, value);
        }
        builder.emit();
    }
}
```

Changes:
- `patchbay/src/handles.rs` — `record()`, `metrics()`, `enter_tracing()`,
  `record_metrics()` (feature-gated)
- `patchbay/src/metrics.rs` — `MetricsBuilder` struct
- `patchbay/src/tracing.rs` — handle `patchbay::_metrics` target, write to
  metrics file. Store `Dispatch` clone in device handle for direct emission
- `patchbay/src/consts.rs` — add `METRICS_JSONL_EXT = "metrics.jsonl"`
- `patchbay/Cargo.toml` — optional `iroh-metrics` dependency
- `patchbay-server/src/lib.rs` — recognize `*.metrics.jsonl` as log kind `metrics`

**LOC estimate:** ~200 new.

### Commit 4: Compare mode

**Goal:** `patchbay compare test main` and `patchbay compare run sims/ main`

Changes:
- `patchbay-cli/src/compare.rs` — new module

  **Worktree management:**
  ```rust
  fn setup_worktree(ref_name: &str, base: &Path) -> Result<PathBuf> {
      let tree_dir = base.join(".patchbay/tree").join(sanitize(ref_name));
      // git worktree add --detach <tree_dir> <ref>
  }
  fn cleanup_if_unchanged(tree_dir: &Path) -> Result<()> {
      // git diff --quiet <tree_dir> && git worktree remove <tree_dir>
  }
  ```

  **Compare test flow (sequential):**
  1. If two refs: create two worktrees. If one ref: worktree + current dir
  2. Run `patchbay test` in left, then right (sequential)
  3. Parse test results from both runs
  4. Write compare batch to `.patchbay/work/compare-{timestamp}/`
     (structured as a batch so it shows in UI naturally)
  5. Print summary table + score
  6. Cleanup unchanged worktrees

  **Compare run flow:**
  Same worktree setup, run sims in each, compare captures/results/metrics.

  **Summary output:**
  ```
  Compare: main ↔ worktree

  Tests:        45/50 pass → 47/50 pass  (+2 fixed)
  Regressions:  0
  New failures: 0
  Total time:   120.3s → 115.1s  (-4.3%)

  ┌──────────────┬────────┬────────┬─────────┐
  │ Test         │ Left   │ Right  │ Delta   │
  ├──────────────┼────────┼────────┼─────────┤
  │ test_nat     │ PASS   │ PASS   │  -0.3s  │
  │ test_relay   │ FAIL   │ PASS   │  fixed  │
  │ test_holepunch│ PASS  │ PASS   │  +0.1s  │
  └──────────────┴────────┴────────┴─────────┘

  Score: +7 (2 fixes, 0 regressions, 4.3% faster)
  ```

  **Scoring formula (simple v0):**
  - +3 per fix (fail→pass)
  - -5 per regression (pass→fail)
  - +1 if total time improves >2%
  - -1 if total time regresses >5%

  **Metrics in compare:** If both sides have `*.metrics.jsonl`, include metric
  deltas in the per-test table (last value of each key compared).

  **qlog in compare (prepared, not implemented):**
  ```rust
  // TODO: qlog comparison — parse per-device qlog files, sum packet/frame
  // counts by type, include as metric deltas in compare summary.
  // See LogsTab.tsx for qlog rendering; comparison adds a delta overlay.
  ```

- `patchbay-cli/src/compare_manifest.rs` — types for compare output

**LOC estimate:** ~350 new.

### Commit 5: UI — metrics view, comparison, tree navigation

**Goal:** Show metrics in UI, add split-screen comparison, improve navigation.

**Architecture:** Extract existing run detail into a reusable `RunView` component,
then compose `CompareView` as two `RunView`s side by side.

Changes:

**Routing overhaul:**
- Switch from hash-based to real URL routes
- Server: wildcard fallback (serve index.html for all non-`/api/`, non-asset paths)
- Routes: `/run/:name`, `/batch/:name`, `/compare/:name`
- Keep `/inv/:name` as redirect to `/batch/:name`

**Navigation — tree selector:**
- Replace `<select>` with a proper tree component in sidebar
- Tree structure: batches (expandable to show runs), standalone runs
- Compare batches show with a compare icon
- Keeps sidebar clean as nesting depth grows

**Refactor — RunView extraction:**
- `ui/src/components/RunView.tsx` — extract from App.tsx
  - Props: `run: RunInfo, state, events, logs, results, metrics`
  - Renders: tab bar + tab content (topology, logs, timeline, perf, metrics)
  - This is a pure extraction, no new behavior

**MetricsTab (new):**
- `ui/src/components/MetricsTab.tsx`
  - Fetches `device.<name>.metrics.jsonl` from run files
  - Parses JSONL, groups by key
  - Default view: table of key + last value, with inline SVG sparklines for
    keys that have multiple data points
  - One row per unique metric key, columns: key, device, last value, sparkline
  - Clicking a row could expand to a full chart (future, not v0)

**CompareView (new):**
- `ui/src/components/CompareView.tsx`
  - Top: `CompareSummary` bar (pass/fail delta, time delta, score badge,
    metrics deltas for shared keys)
  - Below: split panes, left and right
  - Shared tab state — selecting "logs" opens logs on both sides
  - Each side renders a `RunView`
  - Co-navigation: tabs are synchronized, scroll position optionally synced

  ```
  // TODO: qlog comparison — in CompareView summary, show packet/frame count
  // deltas from qlog files. Parse qlog JSON events, bucket by type, diff
  // counts. Display as a compact delta table in CompareSummary.
  ```

**Compare as batch:** Comparison output directory IS a batch. The server discovers
it like any other batch. `CompareView` is activated when the batch has a
`summary.json` (compare manifest). Otherwise it renders as a normal batch view.

**LOC estimate:** ~500 new (200 RunView refactor, 100 MetricsTab, 100 CompareView,
100 tree nav + routing).

---

## Key Patterns

### Backend dispatch (no trait needed for v0)

For simplicity, use a match on `VmBackend` enum rather than a trait:

```rust
enum BackendKind { Native, Qemu, Container }

fn resolve_backend(vm_flag: Option<Option<String>>) -> BackendKind {
    match vm_flag {
        None if cfg!(target_os = "linux") => BackendKind::Native,
        None => auto_detect_vm(),
        Some(None) => auto_detect_vm(),
        Some(Some(s)) => match s.as_str() {
            "qemu" => BackendKind::Qemu,
            "container" => BackendKind::Container,
            _ => bail!("unknown VM backend: {s}"),
        },
    }
}
```

A trait can come later if backends grow; for now a match keeps LOC down.

### Metrics: tracing with JSON payload

Since tracing requires compile-time field names, we use a single `metrics_json`
field containing a serialized map. The patchbay namespace subscriber already has
`JsonFieldVisitor` that extracts fields — we add a branch for the `_metrics`
target that writes to a separate file.

Device holds a clone of its namespace's `tracing::Dispatch` so `record()` and
`metrics().emit()` can emit directly without going through the sync worker thread.
`enter_tracing()` is public so users can also emit arbitrary tracing within the
device context.

### Compare = batch

A compare operation produces a batch directory with left/right runs plus a
`summary.json`. The UI detects `summary.json` and activates `CompareView`.
This means no new API endpoints — compare results flow through existing batch
discovery. The tree navigation component shows compare batches with a visual
indicator.

### UI composition

```
App
├── TreeNav              (new sidebar, replaces <select>)
├── RunView              (extracted, reusable)
│   ├── TopologyGraph
│   ├── LogsTab
│   ├── TimelineTab
│   ├── PerfTab
│   └── MetricsTab       (new)
├── BatchView            (renamed from invocation view)
│   └── RunView per run
└── CompareView          (new, for batches with summary.json)
    ├── CompareSummary
    ├── RunView (left)
    └── RunView (right)
```

Co-navigation: `CompareView` owns the active tab state, passes it down to both
`RunView` instances. Tab clicks update once, both sides follow.

---

## Resolved Questions

1. **nextest:** prefer if installed, else cargo test with warning
2. **Compare parallelism:** sequential for now
3. **Worktree cleanup:** remove if unchanged (git diff empty), keep if modified
4. **Path migration:** minimal warning, no auto-migrate
5. **"invocation" → "batch":** yes, separate commit, keep backward compat routes

---

## Tests

### Integration tests (Rust)

- `patchbay-cli`: test that `patchbay test` invokes cargo test with correct flags
  (mock the cargo command, verify args including `RUSTFLAGS=--cfg patchbay_test`)
- `patchbay-cli`: test that `--vm` flag overrides backend detection
- `patchbay/src/handles.rs`: test `device.record()` writes to metrics file
- `patchbay/src/handles.rs`: test `device.metrics().field().field().emit()`
  writes single line with all fields
- `patchbay-cli/src/compare.rs`: test worktree setup/cleanup, manifest generation
- `patchbay-cli/src/compare.rs`: test scoring formula
- `patchbay-server`: test that metrics.jsonl files are discovered as `metrics` kind

### E2E tests

- **Metrics UI:** playwright test — run a sim that records metrics, verify
  MetricsTab shows key/value table with sparklines
- **Compare UI:** playwright test — create a compare batch directory with mock
  data (left/right runs + summary.json), verify CompareView renders split screen
  with summary bar, co-navigation works (clicking tab changes both sides)
- **Batch rename:** verify `/inv/` routes redirect to `/batch/`
- **Tree nav:** verify sidebar shows batches expandable to runs
