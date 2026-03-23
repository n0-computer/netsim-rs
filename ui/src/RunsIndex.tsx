import { useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { fetchRuns } from './api'
import type { RunInfo, RunManifest } from './api'

interface InvocationGroup {
  invocation: string
  runs: RunInfo[]
  manifest: RunManifest | null
}

function groupByInvocation(runs: RunInfo[]): { groups: InvocationGroup[]; ungrouped: RunInfo[] } {
  const grouped = new Map<string, RunInfo[]>()
  const ungrouped: RunInfo[] = []
  for (const r of runs) {
    if (r.invocation) {
      let list = grouped.get(r.invocation)
      if (!list) {
        list = []
        grouped.set(r.invocation, list)
      }
      list.push(r)
    } else {
      ungrouped.push(r)
    }
  }
  const groups: InvocationGroup[] = []
  for (const [invocation, groupRuns] of grouped) {
    // Use manifest from the first run that has one.
    const manifest = groupRuns.find((r) => r.manifest)?.manifest ?? null
    groups.push({ invocation, runs: groupRuns, manifest })
  }
  return { groups, ungrouped }
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

/** Extract date portion from invocation name like "project-YYYYMMDD_HHMMSS-uuid". */
function extractDate(name: string): string | null {
  const m = name.match(/(\d{8}_\d{6})/)
  return m ? m[1] : null
}

export default function RunsIndex() {
  const [runs, setRuns] = useState<RunInfo[]>([])
  const [loaded, setLoaded] = useState(false)
  const navigate = useNavigate()

  useEffect(() => {
    const refresh = () => fetchRuns().then((r) => { setRuns(r); setLoaded(true) })
    refresh()
    const id = setInterval(refresh, 5_000)
    return () => clearInterval(id)
  }, [])

  const { groups, ungrouped } = groupByInvocation(runs)

  // Auto-navigate: if there's only one run, go directly to it.
  // If there's only one invocation group, go to it.
  useEffect(() => {
    if (!loaded || runs.length === 0) return
    if (runs.length === 1) {
      navigate(`/run/${runs[0].name}`, { replace: true })
    } else if (groups.length === 1 && ungrouped.length === 0) {
      navigate(`/inv/${groups[0].invocation}`, { replace: true })
    }
  }, [loaded, runs, groups, ungrouped, navigate])

  return (
    <div className="runs-index">
      <h1>patchbay runs</h1>
      {runs.length === 0 && loaded && <div className="empty">No runs found.</div>}

      {groups.map((g) => (
        <div key={g.invocation} className="run-group">
          {g.manifest ? (
            <ManifestGroupHeader group={g} />
          ) : (
            <div className="run-group-header">
              <span className="run-group-name">{g.invocation}</span>
              {g.runs.length > 1 && (
                <Link to={`/inv/${g.invocation}`} className="run-link combined">
                  combined ({g.runs.length} sims)
                </Link>
              )}
            </div>
          )}
          {g.runs.map((r) => (
            <RunEntry key={r.name} run={r} grouped />
          ))}
        </div>
      ))}
      {ungrouped.map((r) => (
        <RunEntry key={r.name} run={r} />
      ))}
    </div>
  )
}

function ManifestGroupHeader({ group }: { group: InvocationGroup }) {
  const m = group.manifest!
  const outcome = m.test_outcome
  const statusIcon = outcome === 'success' ? '\u2705' : outcome === 'failure' ? '\u274c' : null
  const date = extractDate(group.invocation)

  return (
    <Link to={`/inv/${group.invocation}`} className="pushed-run-entry">
      <span className="pushed-run-project">{m.project || group.invocation}</span>
      <div className="pushed-run-meta">
        {m.branch && <span className="pushed-run-badge">{m.branch}</span>}
        {m.commit && <code className="pushed-run-sha">{m.commit.slice(0, 7)}</code>}
        {m.pr != null && m.pr_url ? (
          <a
            href={m.pr_url}
            className="pushed-run-pr-link"
            onClick={(e) => e.stopPropagation()}
          >
            PR #{m.pr}
          </a>
        ) : m.pr != null ? (
          <span>PR #{m.pr}</span>
        ) : null}
        {m.title && <span className="pushed-run-title">{m.title}</span>}
      </div>
      <div className="pushed-run-right">
        {statusIcon && <span className="pushed-run-status">{statusIcon}</span>}
        {date && <span className="pushed-run-date">{formatDate(date)}</span>}
        <span className="pushed-run-arrow">&rarr;</span>
      </div>
    </Link>
  )
}

function RunEntry({ run, grouped }: { run: RunInfo; grouped?: boolean }) {
  const label = grouped && run.invocation && run.name.startsWith(run.invocation + '/')
    ? run.label ?? run.name.slice(run.invocation.length + 1)
    : run.label ?? run.name

  return (
    <Link to={`/run/${run.name}`} className="run-entry">
      <span className="run-entry-label">{label}</span>
      {run.status && <span className="run-entry-status">{run.status}</span>}
    </Link>
  )
}
