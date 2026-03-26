import { useMemo } from 'react'
import type { RunInfo } from '../api'
import { groupByGroup, simLabel } from '../utils'

// ── Selection model ────────────────────────────────────────────────

export type Selection =
  | { kind: 'run'; name: string }
  | { kind: 'group'; name: string }

export function selectionKey(s: Selection | null): string {
  if (!s) return ''
  return s.kind === 'group' ? `group:${s.name}` : s.name
}

export function selectionFromValue(val: string): Selection | null {
  if (!val) return null
  if (val.startsWith('group:')) return { kind: 'group', name: val.slice(6) }
  return { kind: 'run', name: val }
}

export function selectionPath(s: Selection | null): string {
  if (!s) return '/'
  return s.kind === 'group' ? `/group/${s.name}` : `/run/${s.name}`
}

// ── Component ──────────────────────────────────────────────────────

interface RunSelectorProps {
  runs: RunInfo[]
  value: Selection | null
  onChange: (selection: Selection | null) => void
}

export default function RunSelector({ runs, value, onChange }: RunSelectorProps) {
  const { groups, ungrouped } = useMemo(() => groupByGroup(runs), [runs])

  return (
    <select
      value={selectionKey(value)}
      onChange={(e) => onChange(selectionFromValue(e.target.value))}
    >
      <option value="">select run</option>
      {groups.map((g) => (
        <optgroup key={g.group} label={g.group}>
          {g.runs.length > 1 && (
            <option value={`group:${g.group}`}>
              combined ({g.runs.length} sims)
            </option>
          )}
          {g.runs.map((r) => (
            <option key={r.name} value={r.name}>
              {simLabel(r)}
            </option>
          ))}
        </optgroup>
      ))}
      {ungrouped.map((r) => (
        <option key={r.name} value={r.name}>
          {r.label ?? r.name}
        </option>
      ))}
    </select>
  )
}
