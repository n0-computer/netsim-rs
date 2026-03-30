import { useEffect, useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { fetchRuns } from './api'
import type { RunInfo, RunManifest } from './api'
import { groupByGroup as groupByGroupBase } from './utils'

// ── Types ──

interface RunGroupWithManifest {
  group: string
  runs: RunInfo[]
  manifest: RunManifest | null
}

// ── Helpers ──

/** Extends the shared groupByGroup with manifest extraction for the index page. */
function groupByGroup(runs: RunInfo[]): { groups: RunGroupWithManifest[]; ungrouped: RunInfo[] } {
  const { groups, ungrouped } = groupByGroupBase(runs)
  return {
    groups: groups.map((g) => ({
      ...g,
      manifest: g.runs.find((r) => r.manifest)?.manifest ?? null,
    })),
    ungrouped,
  }
}

function formatDate(raw: string): string {
  if (raw.length < 15) return raw
  const y = raw.slice(0, 4)
  const mo = raw.slice(4, 6)
  const d = raw.slice(6, 8)
  const h = raw.slice(9, 11)
  const mi = raw.slice(11, 13)
  const s = raw.slice(13, 15)
  return `${y}-${mo}-${d} ${h}:${mi}:${s}`
}

/** Extract date portion from group name like "project-YYYYMMDD_HHMMSS-uuid". */
function extractDate(name: string): string | null {
  const m = name.match(/(\d{8}_\d{6})/)
  return m ? m[1] : null
}

/** Parse a date string (ISO or YYYYMMDD_HHMMSS) to a Date object for sorting. */
function parseDate(s: string): Date {
  // Try ISO format first
  const d = new Date(s)
  if (!isNaN(d.getTime())) return d
  // Try YYYYMMDD_HHMMSS
  const m = s.match(/(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})/)
  if (m) return new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6])
  return new Date(0)
}

/** Get sort key for a run/group - prefer manifest.started_at, fall back to dir name date. */
function sortKey(run: RunInfo): number {
  if (run.manifest?.started_at) return parseDate(run.manifest.started_at).getTime()
  const dateStr = extractDate(run.group ?? run.name)
  if (dateStr) return parseDate(dateStr).getTime()
  return 0
}

/** Format relative time from a date string. */
function relativeTime(dateStr: string): string {
  const d = parseDate(dateStr)
  if (d.getTime() === 0) return ''
  const diff = Date.now() - d.getTime()
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  const days = Math.floor(hrs / 24)
  return `${days}d ago`
}

const PAGE_SIZE = 100

// ── Component ──

export default function RunsIndex() {
  const [runs, setRuns] = useState<RunInfo[]>([])
  const [loaded, setLoaded] = useState(false)
  const navigate = useNavigate()

  // Filters
  const [projectFilter, setProjectFilter] = useState<string>('')
  const [kindFilter, setKindFilter] = useState<string>('')
  const [page, setPage] = useState(0)

  // Checkbox selection for compare
  const [selected, setSelected] = useState<Set<string>>(new Set())
  // Collapsed groups (collapsed by default, expanded set tracks which are open)
  const [expanded, setExpanded] = useState<Set<string>>(new Set())

  useEffect(() => {
    const refresh = () => fetchRuns().then((r) => { setRuns(r); setLoaded(true) })
    refresh()
    const id = setInterval(refresh, 5_000)
    return () => clearInterval(id)
  }, [])

  // Unique projects and kinds for filter dropdowns
  const projects = useMemo(() => {
    const s = new Set<string>()
    for (const r of runs) {
      if (r.manifest?.project) s.add(r.manifest.project)
    }
    return Array.from(s).sort()
  }, [runs])

  const kinds = useMemo(() => {
    const s = new Set<string>()
    for (const r of runs) {
      if (r.manifest?.kind) s.add(r.manifest.kind)
    }
    return Array.from(s).sort()
  }, [runs])

  // Filter and sort runs
  const filteredRuns = useMemo(() => {
    let result = runs
    if (projectFilter) {
      result = result.filter((r) => r.manifest?.project === projectFilter)
    }
    if (kindFilter) {
      result = result.filter((r) => r.manifest?.kind === kindFilter)
    }
    // Sort by date (newest first)
    result = [...result].sort((a, b) => sortKey(b) - sortKey(a))
    return result
  }, [runs, projectFilter, kindFilter])

  // Group filtered runs
  const { groups, ungrouped } = useMemo(() => groupByGroup(filteredRuns), [filteredRuns])

  // Flatten for pagination: each group is one "row", each ungrouped run is one "row"
  type Row = { kind: 'group'; group: RunGroupWithManifest } | { kind: 'run'; run: RunInfo }
  const allRows = useMemo(() => {
    const rows: Row[] = []
    // Sort groups by the first run's sortKey
    const sortedGroups = [...groups].sort((a, b) => {
      const aKey = Math.max(...a.runs.map(sortKey))
      const bKey = Math.max(...b.runs.map(sortKey))
      return bKey - aKey
    })
    // Interleave groups and ungrouped by date
    let gi = 0
    let ui = 0
    while (gi < sortedGroups.length || ui < ungrouped.length) {
      const gKey = gi < sortedGroups.length ? Math.max(...sortedGroups[gi].runs.map(sortKey)) : -1
      const uKey = ui < ungrouped.length ? sortKey(ungrouped[ui]) : -1
      if (gKey >= uKey && gi < sortedGroups.length) {
        rows.push({ kind: 'group', group: sortedGroups[gi] })
        gi++
      } else {
        rows.push({ kind: 'run', run: ungrouped[ui] })
        ui++
      }
    }
    return rows
  }, [groups, ungrouped])

  const totalPages = Math.max(1, Math.ceil(allRows.length / PAGE_SIZE))
  const pageRows = allRows.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE)

  // Reset page when filters change
  useEffect(() => { setPage(0) }, [projectFilter, kindFilter])

  // Toggle a run in the selection set
  const toggleSelected = (name: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }

  const selectedList = Array.from(selected)

  return (
    <div className="runs-index">
      <div style={{ display: 'flex', alignItems: 'center', gap: '1rem', flexWrap: 'wrap', marginBottom: '1rem' }}>
        <h1 style={{ margin: 0 }}>Runs</h1>

        {/* Project filter */}
        {projects.length > 0 && (
          <select value={projectFilter} onChange={(e) => setProjectFilter(e.target.value)} style={filterStyle}>
            <option value="">All projects</option>
            {projects.map((p) => <option key={p} value={p}>{p}</option>)}
          </select>
        )}

        {/* Kind filter */}
        {kinds.length > 0 && (
          <select value={kindFilter} onChange={(e) => setKindFilter(e.target.value)} style={filterStyle}>
            <option value="">All kinds</option>
            {kinds.map((k) => <option key={k} value={k}>{k}</option>)}
          </select>
        )}

        {/* Pagination */}
        <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
          <button disabled={page === 0} onClick={() => setPage(page - 1)} style={navBtnStyle}>&lt; Prev</button>
          <span style={{ fontSize: 12, color: 'var(--text-muted)' }}>{page + 1} / {totalPages}</span>
          <button disabled={page >= totalPages - 1} onClick={() => setPage(page + 1)} style={navBtnStyle}>Next &gt;</button>
        </div>
      </div>

      {/* Compare selected button */}
      {selectedList.length === 2 && (
        <button
          className="compare-selected-btn"
          style={compareBtnStyle}
          onClick={() => {
            navigate(`/compare/${encodeURIComponent(selectedList[0])}/${encodeURIComponent(selectedList[1])}`)
          }}
        >
          Compare Selected ({selectedList.length})
        </button>
      )}

      {runs.length === 0 && loaded && <div className="empty">No runs found.</div>}

      {pageRows.map((row) => {
        if (row.kind === 'group') {
          const g = row.group
          const isExpanded = expanded.has(g.group)
          const toggleExpand = () => setExpanded(prev => {
            const next = new Set(prev)
            if (next.has(g.group)) next.delete(g.group)
            else next.add(g.group)
            return next
          })
          return (
            <div key={g.group} className="run-group">
              <div
                className="run-group-header"
                style={{ display: 'flex', alignItems: 'center', gap: '0.5rem', cursor: 'pointer', padding: '8px 12px' }}
                onClick={toggleExpand}
              >
                <span style={{ fontSize: 12, width: 16, textAlign: 'center', userSelect: 'none' }}>
                  {isExpanded ? '\u25BC' : '\u25B6'}
                </span>
                <input
                  type="checkbox"
                  checked={selected.has(g.group)}
                  aria-label={`Select group ${g.group} for comparison`}
                  onChange={(e) => { e.stopPropagation(); toggleSelected(g.group) }}
                  onClick={(e) => e.stopPropagation()}
                  style={{ cursor: 'pointer' }}
                />
                {g.manifest ? (
                  <GroupHeaderContent group={g} />
                ) : (
                  <Link
                    to={`/group/${g.group}`}
                    className="run-group-name"
                    style={{ color: 'inherit', textDecoration: 'none' }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    {g.group}
                  </Link>
                )}
                <span style={{ fontSize: 11, color: 'var(--text-muted)', marginLeft: 'auto' }}>
                  {g.runs.length} {g.runs.length === 1 ? 'run' : 'runs'}
                </span>
              </div>
              {isExpanded && g.runs.map((r) => (
                <RunRow key={r.name} run={r} grouped selected={selected.has(r.name)} onToggle={toggleSelected} />
              ))}
            </div>
          )
        }
        const r = row.run
        return <RunRow key={r.name} run={r} selected={selected.has(r.name)} onToggle={toggleSelected} />
      })}
    </div>
  )
}

// ── Subcomponents ──

/** Inline content for a group header with manifest info (rendered inside the collapsible header). */
function GroupHeaderContent({ group }: { group: RunGroupWithManifest }) {
  const m = group.manifest!
  const outcome = m.test_outcome ?? m.outcome
  const statusIcon = outcome === 'pass' || outcome === 'success' ? '\u2705' : outcome === 'fail' || outcome === 'failure' ? '\u274c' : null
  const date = m.started_at ?? extractDate(group.group)
  const titleTruncated = m.title && m.title.length > 60 ? m.title.slice(0, 57) + '...' : m.title

  return (
    <>
      <Link
        to={`/group/${group.group}`}
        style={{ fontWeight: 600, color: 'inherit', textDecoration: 'none' }}
        onClick={(e) => e.stopPropagation()}
      >
        {m.project || group.group}
      </Link>
      {m.branch && <span className="pushed-run-badge">{m.branch}</span>}
      {m.commit && <code style={{ fontSize: 11 }}>{m.commit.slice(0, 7)}</code>}
      {m.kind && <span className="kind-badge" style={kindBadgeStyle(m.kind)}>{m.kind}</span>}
      {m.pr != null && m.pr_url && (
        <a
          href={m.pr_url}
          target="_blank"
          rel="noopener noreferrer"
          style={{ fontSize: 11, color: 'var(--accent, #4a9eff)' }}
          onClick={(e) => e.stopPropagation()}
        >
          #{m.pr}
        </a>
      )}
      {m.pr != null && !m.pr_url && (
        <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>#{m.pr}</span>
      )}
      {titleTruncated && <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>{titleTruncated}</span>}
      {statusIcon && <span>{statusIcon}</span>}
      {date && (
        <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
          {typeof date === 'string' && date.includes('T') ? relativeTime(date) : formatDate(date as string)}
        </span>
      )}
      {m.pass != null && m.total != null && (
        <span style={{ fontSize: 12 }}>{m.pass}/{m.total} pass</span>
      )}
    </>
  )
}

function RunRow({ run, grouped, selected, onToggle }: { run: RunInfo; grouped?: boolean; selected: boolean; onToggle: (name: string) => void }) {
  // For grouped (child) runs: show the test/sim name, not the inherited manifest info
  const shortName = grouped && run.group && run.name.startsWith(run.group + '/')
    ? run.name.slice(run.group.length + 1).replace(/\//g, ' / ')
    : null
  const displayLabel = shortName ?? run.label ?? run.name

  // Only show manifest details for ungrouped (top-level) runs
  const m = grouped ? null : run.manifest

  const branchCommit = m?.branch && m?.commit
    ? `${m.branch}@${m.commit.slice(0, 7)}`
    : m?.commit ? m.commit.slice(0, 7)
    : null

  const dateStr = m?.started_at ?? extractDate(run.group ?? run.name)
  const kindBadge = grouped ? null : m?.kind

  return (
    <div className="run-entry" style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
      <input
        type="checkbox"
        checked={selected}
        aria-label={`Select ${displayLabel} for comparison`}
        onChange={(e) => { e.stopPropagation(); onToggle(run.name) }}
        onClick={(e) => e.stopPropagation()}
        style={{ cursor: 'pointer' }}
      />
      <Link to={`/run/${run.name}`} style={{ flex: 1, display: 'flex', alignItems: 'center', gap: '0.5rem', color: 'inherit', textDecoration: 'none' }}>
        <span className="run-entry-label" style={{ flex: 1 }}>
          {branchCommit ?? displayLabel}
        </span>
        {kindBadge && (
          <span className="kind-badge" style={kindBadgeStyle(kindBadge)}>{kindBadge}</span>
        )}
        {!grouped && dateStr && (
          <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
            {typeof dateStr === 'string' && dateStr.includes('T') ? relativeTime(dateStr) : dateStr}
          </span>
        )}
        {!grouped && m?.pass != null && m?.total != null && (
          <span style={{ fontSize: 12 }}>
            {m.pass}/{m.total} pass
          </span>
        )}
        {run.status && <span className="run-entry-status">{run.status}</span>}
      </Link>
    </div>
  )
}

// ── Styles ──

const filterStyle: React.CSSProperties = {
  padding: '4px 8px',
  borderRadius: 4,
  border: '1px solid var(--border)',
  background: 'var(--surface)',
  color: 'inherit',
  fontSize: 13,
}

const navBtnStyle: React.CSSProperties = {
  padding: '4px 8px',
  borderRadius: 4,
  border: '1px solid var(--border)',
  background: 'var(--surface)',
  color: 'inherit',
  fontSize: 12,
  cursor: 'pointer',
}

const compareBtnStyle: React.CSSProperties = {
  padding: '8px 16px',
  borderRadius: 6,
  border: 'none',
  background: 'var(--accent, #4a9eff)',
  color: '#fff',
  fontWeight: 'bold',
  cursor: 'pointer',
  marginBottom: '1rem',
}

function kindBadgeStyle(kind: string): React.CSSProperties {
  return {
    fontSize: 10,
    padding: '2px 6px',
    borderRadius: 4,
    fontWeight: 600,
    textTransform: 'uppercase',
    background: kind === 'test' ? 'rgba(74, 158, 255, 0.15)' : 'rgba(255, 158, 74, 0.15)',
    color: kind === 'test' ? 'var(--accent, #4a9eff)' : '#ff9e4a',
  }
}
