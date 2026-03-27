# Compare & Run Data Model Refactor

## Problem Statement

The current compare implementation creates separate `compare-{timestamp}/` directories with
a `summary.json` that duplicates test results. Runs and tests lack a unified manifest, making
it impossible to find "the run for commit X" or compare arbitrary runs in the UI. Three
different `RunManifest` structs exist across crates. The naming is inconsistent (`batch`,
`invocation`, `sim-` prefix for what is actually a "run").

This refactor unifies the data model so that:
- Every execution (test or sim) writes a `run.json` manifest with git context
- Compare is a view over two existing runs, not a separate artifact
- The UI can compare any two runs from the same project
- `patchbay compare` is smart about caching (skip if run for that ref already exists)

## Naming

Everything is a **run**. A run has a `kind` field (enum: `Test` or `Sim`).

| Term | Meaning |
|------|---------|
| **run** | Any single execution. The atomic unit everywhere. |
| **kind** | `RunKind::Test` or `RunKind::Sim` — what produced the run |
| **group** | When `patchbay run sims/` processes N sim TOMLs, the top-level `run-{timestamp}/` dir is the group. Each sim inside is a nested run. For tests, each test binary's output under `testdir-current/` is a nested run (if it has `events.jsonl`). The group shares the `run.json` manifest. |
| **project** | A named scope for filtering & comparing (e.g. `"iroh"`) |

"batch" is retired (kept as serde alias for backward compat).

### Directory naming

| Context | Current | New |
|---------|---------|-----|
| Sim run root | `sim-YYMMDD-HHMMSS/` | `run-YYMMDD-HHMMSS/` |
| Pushed run | `{project}-{date}-{uuid}/` | unchanged |
| Compare dir | `compare-{timestamp}/` | **removed** (compare is computed on the fly) |
| Worktree | `.patchbay/tree/{ref}/` | unchanged |
| VM state | `.patchbay/vm/` | unchanged |
| Image cache | `~/.local/share/patchbay/qemu-images/` | unchanged (stays in XDG) |

### Testdir nesting

`testdir!()` creates nested subdirectories for module paths:
`testdir-current/crate_name/module/test_name/`. The server scans up to 3 levels deep
for `events.jsonl`, so nested Lab output is discovered automatically. This is fine as-is.

## Unified `run.json` Manifest

One struct, defined in `patchbay-utils/src/manifest.rs` (shared between runner, CLI, server):

```rust
use chrono::{DateTime, Utc};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunKind {
    Test,
    Sim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Pass,
    Fail,
    Ignored,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    /// Test duration, serialized as integer milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "option_duration_ms")]
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    // ── Identity ──
    pub kind: RunKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    // ── Git context ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub dirty: bool,

    // ── CI context (populated from env vars when available) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    // ── Execution ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// Total runtime, serialized as integer milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "option_duration_ms")]
    pub runtime: Option<Duration>,

    // ── Outcome ──
    /// "pass" or "fail". Aliases for backward compat with old run.json fields.
    #[serde(default, skip_serializing_if = "Option::is_none",
            alias = "test_outcome", alias = "status")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,

    // ── Per-test results (kind == Test only) ──
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<TestResult>,

    // ── Environment ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patchbay_version: Option<String>,
}
```

### Duration serialization module (in patchbay-utils)

Move the existing `duration_ms` serde module from `compare.rs` to `patchbay-utils/src/manifest.rs`.
Add an `option_duration_ms` variant for `Option<Duration>` fields.

```rust
pub mod duration_ms {
    use std::time::Duration;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

pub mod option_duration_ms {
    use std::time::Duration;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_u64(d.as_millis() as u64),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?.map(Duration::from_millis))
    }
}
```

### Git context helper (in patchbay-utils)

```rust
pub struct GitContext {
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub dirty: bool,
}

pub fn git_context() -> GitContext {
    let commit = Command::new("git").args(["rev-parse", "HEAD"]).output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());
    let branch = Command::new("git").args(["rev-parse", "--abbrev-ref", "HEAD"]).output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| s != "HEAD");
    let dirty = !Command::new("git").args(["diff", "--quiet"]).status()
        .map(|s| s.success()).unwrap_or(true);
    GitContext { commit, branch, dirty }
}

/// Resolve a git ref (branch name, tag, or SHA prefix) to a full commit SHA.
pub fn resolve_ref(git_ref: &str) -> Option<String> {
    Command::new("git").args(["rev-parse", git_ref]).output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}
```

### Who writes `run.json`

| Command | Where | kind | Git info |
|---------|-------|------|----------|
| `patchbay test` | testdir output root | `Test` | `git_context()` |
| `patchbay test --persist` | also copies to `.patchbay/work/run-{ts}/` | `Test` | same |
| `patchbay run` | `run_root` (`run-{ts}/` dir) | `Sim` | `git_context()` |
| `patchbay upload` | writes if missing, reads if present | either | from CI env vars |
| `patchbay compare test` | each worktree test run writes its own | `Test` | worktree HEAD |

## How `patchbay run` changes

`prepare_run_root` creates `run-{timestamp}/` (rename from `sim-{timestamp}/`).

After all sims finish, write `run.json` alongside the existing `manifest.json`.
Rename the runner's `RunManifest` (in `progress.rs`) → `SimRunReport` to avoid collision.
Both files coexist for now; `manifest.json` has per-sim details, `run.json` has unified metadata.
Docs clarify the distinction. Long-term merge target.

### Group semantics for sim runs

When `patchbay run sims/` processes multiple TOML files, the `run-{timestamp}/` directory is
the group. Each sim inside (`my-sim/`, `my-sim-2/`) is a nested run with its own `events.jsonl`.
The server discovers nested runs and derives `group` from the first path component.
The group-level `run.json` provides the shared manifest.

## How `patchbay test` changes

After `cargo test` / `nextest` finishes:

1. Pipe stdout/stderr, parse test output via `parse_test_output()` for per-test results
2. Locate `testdir-current/` via cargo metadata
3. Also set `PATCHBAY_OUTDIR` env var so Labs write to a known location
4. Write `run.json` into testdir-current/ (or PATCHBAY_OUTDIR if it was used and is non-empty)
5. If `--persist` flag is set, copy the whole thing into `.patchbay/work/run-{ts}/`

### testdir and PATCHBAY_OUTDIR

`patchbay test` sets `PATCHBAY_OUTDIR` when running cargo test. After the test finishes:
- If the PATCHBAY_OUTDIR directory exists and is non-empty → use it for run.json
- Otherwise check if testdir-current exists → use that
- Write `run.json` with kind, git context, parsed test results

Consider re-exporting `testdir` from the patchbay crate (`patchbay::testdir`) for convenience.

## How `patchbay compare` changes

### New flow

```
patchbay compare test <ref> [ref2] [-- test-args]
  --force-build     Force rebuild even if cached run exists
  --no-ref-build    Don't build; fail if no cached run found
```

1. **Resolve refs to commits**: `resolve_ref(ref)` → full SHA
2. **Check for cached runs**: `find_run_for_commit(".patchbay/work", sha, RunKind::Test)`
   scans `*/run.json` for matching `commit` + `kind` + `dirty == false`
3. **For each ref without a cached run**:
   - If `--no-ref-build`: fail with "no run found for {ref}, use --force-build"
   - Create worktree at `.patchbay/tree/{ref}/`
   - Run `cargo test` in worktree, persist results to `.patchbay/work/run-{ts}/`
   - Clean up worktree
4. **For current worktree** (when ref2 is omitted):
   - Check if a run exists for HEAD (match `dirty` against current state)
   - If not, run tests and persist
5. **If `--force-build`**: skip cache check, always build & run
6. **Diff**: Load both `run.json` manifests, compare `tests` arrays
   - Print summary table + score
   - Exit non-zero on regressions

### Cached run lookup

```rust
/// Find a persisted run matching commit SHA and kind.
pub fn find_run_for_commit(work_dir: &Path, commit: &str, kind: RunKind) -> Option<(PathBuf, RunManifest)> {
    for entry in fs::read_dir(work_dir).ok()?.flatten() {
        let run_json = entry.path().join("run.json");
        if let Ok(text) = fs::read_to_string(&run_json) {
            if let Ok(m) = serde_json::from_str::<RunManifest>(&text) {
                if m.kind == kind && m.commit.as_deref() == Some(commit) && !m.dirty {
                    return Some((entry.path(), m));
                }
            }
        }
    }
    None
}
```

### Comparing two RunManifests

`compare_results()` takes two `&RunManifest`s and returns a computed summary. Uses the `tests`
field for per-test diff. Same scoring logic as before (fixes +3, regressions -5, time delta ±1).

No `CompareManifest` or `CompareSummary` structs stored to disk. The summary is printed to
stdout and optionally returned as JSON for the UI.

## Server changes

### Discovery

Extend `discover_runs` to detect directories with `run.json` in addition to `events.jsonl`:
```rust
if path.join(EVENTS_JSONL).exists() || path.join(RUN_JSON).exists() {
    // This is a run
}
```

### RunInfo changes

```rust
pub struct RunInfo {
    pub name: String,
    #[serde(skip)]
    pub path: PathBuf,
    pub label: Option<String>,
    pub status: Option<String>,
    /// Group name (first path component for nested runs).
    /// Serialized as both "group" and legacy "batch".
    #[serde(alias = "batch")]
    pub group: Option<String>,
    pub manifest: Option<RunManifest>,  // unified RunManifest from patchbay-utils
}
```

### API changes

- `GET /api/runs` gains optional query params: `?project=X&kind=test&limit=100&offset=0`
- Response includes `group` field (with `batch` as serde alias)
- Keep `/api/batches/` and `/api/invocations/` routes as aliases
- Compare is computed client-side (no new server endpoint needed)

## UI changes

### Runs index redesign

Single page at `/`:

```
┌──────────────────────────────────────────────────────┐
│ Runs           [Project ▾] [Kind ▾]     [< 1 2 3 >] │
│                                                       │
│ ☐ main@abc123  test  2m ago   47/50 pass    [view]   │
│ ☐ main@def456  test  1h ago   45/50 pass    [view]   │
│ ☐ feat@789abc  sim   3h ago   pass          [view]   │
│                                                       │
│ [Compare Selected (2)]                                │
└──────────────────────────────────────────────────────┘
```

- **Sorted by date** (newest first, from `started_at` or dir name)
- **Project filter** dropdown (populated from unique `manifest.project` values)
- **Kind filter** dropdown (test/sim/all)
- **Pagination** (100 per page)
- **Checkboxes** for multi-select → "Compare Selected" button
- Click row → `/run/{name}` detail view
- Grouped runs (multi-sim) show as expandable rows

### Compare view

Route: `/compare/:left/:right`

- Fetch both runs' `run.json` via `/api/runs/{name}/files/run.json`
- Compute diff client-side (same logic as CLI compare)
- Summary bar: left ref, right ref, pass/fail counts, score
- Per-test table: name, left status, right status, delta badge
- Side-by-side metrics (if metrics.jsonl exists in both)
- "Compare with..." button on individual run pages (picker shows same-project runs)

### Co-navigation

Split-screen layout reusing RunView for each side. Shared tab state — switching tab on
one side switches both. Scroll sync optional (defer to v2).

### Router

```
/                           → RunsIndex
/run/:name                  → RunDetail (single run view)
/compare/:left/:right       → CompareView (side-by-side)
/batch/:name                → alias for group view
/inv/:name                  → redirect to /batch/:name (legacy)
```

## Implementation Phases

### Phase 1: Unified RunManifest + run.json everywhere

**Commit 1a: Move manifest types to patchbay-utils**
- Create `patchbay-utils/src/manifest.rs`
- Define `RunKind`, `TestStatus`, `TestResult`, `RunManifest`, `GitContext`
- Move `duration_ms` / `option_duration_ms` serde modules there
- Add `git_context()`, `resolve_ref()`, `find_run_for_commit()` helpers
- Export from `patchbay-utils/src/lib.rs`
- Add `chrono` dependency to patchbay-utils (workspace dep)
- Delete duplicate RunManifest from `patchbay-cli/src/upload.rs`
- Delete duplicate RunManifest from `patchbay-server/src/lib.rs`
- Import from `patchbay_utils::manifest::*` in both
- Server: keep backward-compat serde aliases
- Files changed: `patchbay-utils/{Cargo.toml, src/lib.rs, src/manifest.rs}`,
  `patchbay-cli/src/upload.rs`, `patchbay-server/src/lib.rs`,
  `patchbay-cli/src/compare.rs` (delete TestResult/TestStatus, import from utils)

**Commit 1b: Rename runner's RunManifest → SimRunReport**
- In `patchbay-runner/src/sim/progress.rs`: rename `RunManifest` → `SimRunReport`
- Update all references in `runner.rs`
- Add doc comments distinguishing from the unified `run.json` manifest
- Files changed: `patchbay-runner/src/sim/progress.rs`, `patchbay-runner/src/sim/runner.rs`

**Commit 1c: `patchbay run` writes run.json**
- In `runner.rs::run_sims()`, after writing `manifest.json`, also write `run.json`
  using `patchbay_utils::manifest::RunManifest` with `kind: Sim`
- Rename dir prefix `sim-` → `run-` in `prepare_run_root()`
- Files changed: `patchbay-runner/src/sim/runner.rs`

**Commit 1d: `patchbay test` writes run.json + --persist**
- Pipe stdout/stderr from cargo test, parse with `parse_test_output()`
- Write `run.json` to testdir-current (or PATCHBAY_OUTDIR) with test results
- Add `--persist` flag to Test command: copies output dir to `.patchbay/work/run-{ts}/`
- Set `PATCHBAY_OUTDIR` env var when running cargo test
- Files changed: `patchbay-cli/src/test.rs`, `patchbay-cli/src/main.rs`

### Phase 2: Refactor compare to use cached runs

**Commit 2a: Compare uses run.json + cache lookup**
- Rewrite compare flow: resolve refs → check cache → build if needed → diff run.json
- Delete `CompareManifest`, `CompareSummary` structs (compare is computed, not stored)
- `compare_results()` takes two `&RunManifest` and returns printed summary
- Add `--force-build` and `--no-ref-build` flags
- Remove `compare-{timestamp}/` directory creation
- Files changed: `patchbay-cli/src/compare.rs`, `patchbay-cli/src/main.rs`

### Phase 3: Server + API updates

**Commit 3a: Server discovers run.json + group rename**
- Extend `discover_runs` to check for `run.json` in addition to `events.jsonl`
- Rename `batch` → `group` in `RunInfo` (keep `batch` as serde alias)
- Import `RunManifest` from patchbay-utils instead of local definition
- Add query params to `GET /api/runs`: `project`, `kind`, `limit`, `offset`
- Files changed: `patchbay-server/src/lib.rs`

**Commit 3b: Rename batch → group in UI types**
- Update `api.ts`, `RunsIndex.tsx`, `App.tsx` to use `group` (keep `batch` as fallback)
- Files changed: `ui/src/api.ts`, `ui/src/RunsIndex.tsx`, `ui/src/App.tsx`

### Phase 4: UI overhaul

**Commit 4a: Runs index redesign**
- Project dropdown filter, kind dropdown filter
- Pagination (100/page)
- Checkbox selection for compare
- Sorted by date (from manifest.started_at or dir name)
- Files changed: `ui/src/RunsIndex.tsx`, `ui/src/api.ts` (add query params)

**Commit 4b: Compare view refactor**
- New route: `/compare/:left/:right`
- Fetch both runs' `run.json`, compute diff client-side
- Summary bar, per-test table with delta badges, score
- "Compare with..." button on individual run pages
- Files changed: `ui/src/components/CompareView.tsx`, `ui/src/main.tsx`, `ui/src/App.tsx`

**Commit 4c: Co-navigation (side-by-side)**
- Split-screen layout reusing RunView for each side
- Shared tab state (switching tab on one side switches both)
- Files changed: `ui/src/components/CompareView.tsx`

### Phase 5: Tests

**Commit 5a: Update integration test**
- Rewrite `compare_integration.rs` for new flow (no compare directory, reads run.json)
- Fixture crate runs via `patchbay test --persist`
- Assert cached run lookup works (second compare skips build)
- Files changed: `patchbay-cli/tests/compare_integration.rs`

**Commit 5b: Update E2E test**
- Rewrite `compare.spec.ts` for new routes and data model
- Mock two run directories with `run.json` manifests containing test results
- Assert compare view renders from `/compare/run-a/run-b`
- Files changed: `ui/e2e/compare.spec.ts`

## Key invariants

1. `run.json` is the single source of truth for run metadata
2. Filesystem is the only source of truth for the server (no persistent index)
3. Compare is always computed, never stored
4. Every `patchbay test --persist` and `patchbay run` produces a discoverable run
5. Image cache stays in `~/.local/share/patchbay/` (XDG), not `.patchbay/`
6. Backward compat: old `batch`, `test_outcome`, `status` fields still deserialize

## Decisions

1. **manifest.json vs run.json**: Both coexist. `manifest.json` (SimRunReport) has per-sim
   details. `run.json` (RunManifest) has unified metadata. Naming and docs are clear.
   Long-term merge target.

2. **Pagination**: offset/limit (file-based discovery is inherently bounded).

3. **testdir**: Supported mechanism for test output. Consider re-exporting from `patchbay::testdir`.
   Also set `PATCHBAY_OUTDIR` and check both locations.
