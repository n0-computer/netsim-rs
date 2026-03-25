import { useEffect, useState } from 'react'
import { runFilesBase } from '../api'

interface CompareManifest {
  left_ref: string
  right_ref: string
  timestamp: string
  summary: {
    left: { pass: number; fail: number; total: number; time: number }
    right: { pass: number; fail: number; total: number; time: number }
    fixes: number; regressions: number; score: number
  }
  left_results: { name: string; status: string; duration_ms?: number }[]
  right_results: { name: string; status: string; duration_ms?: number }[]
}

export default function CompareView({ batchName }: { batchName: string }) {
  const [manifest, setManifest] = useState<CompareManifest | null>(null)

  useEffect(() => {
    fetch(`${runFilesBase(batchName)}summary.json`)
      .then(r => r.ok ? r.json() : null)
      .then(setManifest)
      .catch(() => setManifest(null))
  }, [batchName])

  if (!manifest) return <div className="empty">Loading compare data...</div>

  const { summary: s } = manifest
  const allTests = new Map<string, { left?: string; right?: string }>()
  for (const r of manifest.left_results) {
    allTests.set(r.name, { left: r.status })
  }
  for (const r of manifest.right_results) {
    const entry = allTests.get(r.name) || {}
    entry.right = r.status
    allTests.set(r.name, entry)
  }

  return (
    <div style={{ padding: '1rem' }}>
      <h2>Compare: {manifest.left_ref} vs {manifest.right_ref}</h2>

      {/* Summary bar */}
      <div className="compare-summary" style={{ display: 'flex', gap: '2rem', padding: '1rem', background: 'var(--surface)', borderRadius: '8px', marginBottom: '1rem', border: '1px solid var(--border)' }}>
        <div>
          <strong>Tests:</strong> {s.left.pass}/{s.left.total} &rarr; {s.right.pass}/{s.right.total}
        </div>
        {s.fixes > 0 && <div style={{ color: 'var(--green)' }}>Fixes: {s.fixes}</div>}
        {s.regressions > 0 && <div style={{ color: 'var(--red)' }}>Regressions: {s.regressions}</div>}
        <div>
          Score: <span style={{ color: s.score >= 0 ? 'var(--green)' : 'var(--red)', fontWeight: 'bold' }}>
            {s.score >= 0 ? '+' : ''}{s.score}
          </span>
        </div>
      </div>

      {/* Per-test table */}
      <div className="tbl-wrap">
        <table>
          <thead>
            <tr>
              <th>Test</th>
              <th>{manifest.left_ref}</th>
              <th>{manifest.right_ref}</th>
              <th>Delta</th>
            </tr>
          </thead>
          <tbody>
            {Array.from(allTests.entries()).sort(([a], [b]) => a.localeCompare(b)).map(([name, { left, right }]) => {
              let delta = ''
              let color = ''
              if (left === 'fail' && right === 'pass') { delta = 'fixed'; color = 'var(--green)' }
              else if (left === 'pass' && right === 'fail') { delta = 'REGRESS'; color = 'var(--red)' }
              else if (!left) { delta = 'new' }
              else if (!right) { delta = 'removed' }

              return (
                <tr key={name}>
                  <td><code>{name}</code></td>
                  <td>{statusBadge(left)}</td>
                  <td>{statusBadge(right)}</td>
                  <td style={{ color, fontWeight: delta ? 'bold' : 'normal' }}>{delta}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function statusBadge(status?: string) {
  if (!status) return <span style={{ color: 'var(--text-muted)' }}>&#8212;</span>
  const colors: Record<string, string> = {
    pass: 'var(--green)',
    fail: 'var(--red)',
    ignored: 'var(--text-muted)',
  }
  return <span style={{ color: colors[status] || 'inherit' }}>{status.toUpperCase()}</span>
}
