# Compare & Run Discovery Cleanup Plan

## Status Quo Issues

### 1. No group page
The CI `view_url` links to `/run/{group-id}` but RunPage expects a leaf run with `events.jsonl`. Groups have no page — they're just collapsible headers in the index. Need a dedicated group page showing: manifest metadata (PR, branch, commit, outcome), list of child test runs with pass/fail, link to compare with other groups.

### 2. Group compare vs run compare conflated
`CompareView` handles both in one component. Group compare should show a test diff table. Run compare should show side-by-side logs/topology. Currently both are always rendered, with the split view trying to load events from a group directory (which has none).

### 3. Test names from nextest vs directory names
Nextest outputs `iroh::patchbay$change_ifaces`. Testdir creates `patchbay/change_ifaces/`. The `resolve_test_dirs` function tries to bridge this but the approach is backwards — it starts from nextest names and tries to find dirs. Should start from dirs and enrich with nextest metadata.

### 4. Non-clickable test links in group compare
Even when dirs exist on disk, `dir` fields are relative to the group (`patchbay/holepunch_simple`) but the compare URL needs the full path (`group-id/patchbay/holepunch_simple`).

### 5. PR/title not shown in index or group page
`run.json` has `pr`, `pr_url`, `title` but the index only shows branch and commit. No PR link or number displayed.

### 6. Index links go to wrong pages
Group headers don't link anywhere. Should link to `/group/{name}`. Individual runs should link to `/run/{name}`.

## Design

### Principle: directories are truth, nextest enriches

- **Group** = directory with `run.json` + child dirs containing `events.jsonl`
- **Run** (leaf) = directory with `events.jsonl`
- Test names in UI = directory paths (e.g. `patchbay/holepunch_simple`)
- Nextest adds status/duration for tests without testdirs (compile failures, skipped)

### Routes

```
/                              RunsIndex (groups + ungrouped runs)
/group/{name}                  GroupPage (manifest + child runs + compare button)
/run/{name}                    RunPage (leaf run: topology/logs/timeline/metrics)
/compare/:left/:right          CompareView (auto-detect group vs run)
```

## Implementation

### Commit 1: `build_test_list` replaces `resolve_test_dirs`

**patchbay-utils/src/manifest.rs**

Rename and rewrite `resolve_test_dirs` → `build_test_list`:
1. Scan child dirs for `events.jsonl` via `collect_event_dirs` → get dir paths
2. Create `TestResult` for each dir: `name = dir path`, `dir = dir path`, `status = Pass`
3. For each nextest test in `self.tests`: extract bare fn name (after `$` or `::`), find matching dir entry, update its `status` and `duration`
4. Nextest tests with NO matching dir → append with nextest name (no `dir`)
5. Replace `self.tests` with combined list

**patchbay-server/src/lib.rs**

In `get_run_manifest`: after `read_run_json` (which calls `build_test_list`), if the directory is a group (no `events.jsonl`), prefix all `test.dir` values with the run name from the URL path. This makes dirs full paths for the UI.

### Commit 2: GroupPage component

**ui/src/GroupPage.tsx** (new)

Route: `/group/:name`

Fetches:
- `fetchRunJson(name)` → manifest with PR/branch/commit/outcome/tests
- `fetchRuns()` filtered by `group === name` → child runs

Renders:
- Header: project, branch@commit, PR link (#number), outcome badge, title
- Test results table: name, status, duration, link to `/run/{full-path}`
- "Compare with..." button → opens a picker of other groups from same project
- Pass/fail summary bar

**ui/src/main.tsx**: add route `/group/:name` → `<GroupPage />`

**patchbay-server/src/lib.rs**: add `/group/{*rest}` to SPA fallback routes (already exists, verify)

### Commit 3: Clean up CompareView

**ui/src/components/CompareView.tsx**

Split rendering:
- `isGroup = true` → render ONLY diff table (no SplitRunView)
- `isGroup = false` → render ONLY SplitRunView (no diff table)

Remove `manifestFromGroup()` — after commit 1, `fetchRunJson(groupName)` returns a proper manifest with dir-based test names and full-path dirs. Keep as fallback only for groups without `run.json` (local testdir serving).

`handleTestClick` uses `leftDir`/`rightDir` directly (already full paths after commit 1).

### Commit 4: Index improvements

**ui/src/RunsIndex.tsx**

- Group header links to `/group/{name}` (clickable)
- Show PR number + link when `manifest.pr` exists: `#3986` linked to `manifest.pr_url`
- Show `manifest.title` (truncated) next to branch
- Individual runs link to `/run/{name}`
- Pass/fail counts shown inline: `18/24 pass` with color

### Commit 5: Push `view_url` points to group page

**patchbay-server/src/lib.rs** (push_run)

Change `view_url` construction to use `/group/` prefix instead of `/run/`:
```rust
let view_url = format!("{origin}/group/{run_name}");
```

This way CI links go directly to the group page.

## What NOT to change

- CI workflow template (correct)
- Push endpoint logic (correct)
- `run.json` on-disk format (backward compat)
- `parse_nextest_json` (correct)
- TimelineTab (already fixed)
