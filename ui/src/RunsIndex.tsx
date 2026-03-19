import { useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { fetchRuns, subscribeRuns } from './api'
import type { RunInfo } from './api'

interface InvocationGroup {
  invocation: string
  runs: RunInfo[]
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
    groups.push({ invocation, runs: groupRuns })
  }
  return { groups, ungrouped }
}

export default function RunsIndex() {
  const [runs, setRuns] = useState<RunInfo[]>([])
  const [loaded, setLoaded] = useState(false)
  const navigate = useNavigate()

  useEffect(() => {
    const refresh = () => fetchRuns().then((r) => { setRuns(r); setLoaded(true) })
    refresh()
    const es = subscribeRuns(refresh)
    return () => es.close()
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
          <div className="run-group-header">
            <span className="run-group-name">{g.invocation}</span>
            {g.runs.length > 1 && (
              <Link to={`/inv/${g.invocation}`} className="run-link combined">
                combined ({g.runs.length} sims)
              </Link>
            )}
          </div>
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
