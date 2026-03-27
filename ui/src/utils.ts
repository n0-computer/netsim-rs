import type { RunInfo } from './api'

// ── Group helpers ───────────────────────────────────────────────────

export interface RunGroup {
  group: string
  runs: RunInfo[]
}

export function groupByGroup(runs: RunInfo[]): { groups: RunGroup[]; ungrouped: RunInfo[] } {
  const grouped = new Map<string, RunInfo[]>()
  const ungrouped: RunInfo[] = []
  for (const r of runs) {
    if (r.group) {
      let list = grouped.get(r.group)
      if (!list) {
        list = []
        grouped.set(r.group, list)
      }
      list.push(r)
    } else {
      ungrouped.push(r)
    }
  }
  const groups: RunGroup[] = []
  for (const [group, groupRuns] of grouped) {
    groups.push({ group, runs: groupRuns })
  }
  return { groups, ungrouped }
}

/** Short display label for a run within a group. */
export function simLabel(run: RunInfo): string {
  if (run.group && run.name.startsWith(run.group + '/')) {
    return run.label ?? run.name.slice(run.group.length + 1)
  }
  return run.label ?? run.name
}
