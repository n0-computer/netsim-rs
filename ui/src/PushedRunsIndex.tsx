import { useEffect, useState } from 'react'
import { fetchPushedRuns } from './api'
import type { PushedRunEntry } from './api'

function formatDate(raw: string): string {
  // raw is "YYYYMMDD_HHMMSS"
  if (raw.length < 15) return raw
  const y = raw.slice(0, 4)
  const mo = raw.slice(4, 6)
  const d = raw.slice(6, 8)
  const h = raw.slice(9, 11)
  const mi = raw.slice(11, 13)
  const s = raw.slice(13, 15)
  return `${y}-${mo}-${d} ${h}:${mi}:${s}`
}

export default function PushedRunsIndex() {
  const [entries, setEntries] = useState<PushedRunEntry[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    const refresh = () =>
      fetchPushedRuns().then((r) => {
        setEntries(r)
        setLoaded(true)
      })
    refresh()
    const id = setInterval(refresh, 10_000)
    return () => clearInterval(id)
  }, [])

  return (
    <div className="pushed-runs-index">
      <h1>patchbay runs</h1>
      {entries.length === 0 && loaded && (
        <div className="empty">No runs yet. Push results using the API.</div>
      )}
      {entries.map((entry) => {
        const m = entry.manifest
        const status = m?.status
        const statusIcon = status === 'success' ? '\u2705' : status === 'failure' ? '\u274c' : null
        return (
          <a
            key={entry.path}
            className="pushed-run-entry"
            href={`/#/inv/${entry.path}`}
          >
            <span className="pushed-run-project">{entry.project}</span>
            <div className="pushed-run-meta">
              {m?.branch && <span className="pushed-run-badge">{m.branch}</span>}
              {m?.commit && <code className="pushed-run-sha">{m.commit.slice(0, 7)}</code>}
              {m?.pr != null && m?.pr_url ? (
                <a
                  href={m.pr_url}
                  className="pushed-run-pr-link"
                  onClick={(e) => e.stopPropagation()}
                >
                  PR #{m.pr}
                </a>
              ) : m?.pr != null ? (
                <span>PR #{m.pr}</span>
              ) : null}
              {m?.title && <span className="pushed-run-title">{m.title}</span>}
            </div>
            <div className="pushed-run-right">
              {statusIcon && <span className="pushed-run-status">{statusIcon}</span>}
              {entry.date && (
                <span className="pushed-run-date">{formatDate(entry.date)}</span>
              )}
              <span className="pushed-run-arrow">&rarr;</span>
            </div>
          </a>
        )
      })}
    </div>
  )
}
