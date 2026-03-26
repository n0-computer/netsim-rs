import { useEffect, useState } from 'react'
import { fetchRunJson, fetchState, fetchEvents, fetchLogs, fetchResults } from '../api'
import type { RunManifest, RunInfo, LogEntry } from '../api'
import RunView from './RunView'
import type { RunTab } from './RunView'

// ── Scoring (same as CLI: fixes +3, regressions -5) ──

const SCORE_FIX = 3
const SCORE_REGRESS = -5

interface TestDelta {
  name: string
  left?: string
  right?: string
  delta: 'fixed' | 'REGRESS' | 'new' | 'removed' | ''
}

function computeDiff(left: RunManifest, right: RunManifest) {
  const leftTests = left.tests ?? []
  const rightTests = right.tests ?? []
  const leftMap = new Map(leftTests.map(t => [t.name, t.status]))
  const rightMap = new Map(rightTests.map(t => [t.name, t.status]))

  const allNames = new Set([...leftMap.keys(), ...rightMap.keys()])
  const tests: TestDelta[] = []
  let fixes = 0
  let regressions = 0

  for (const name of Array.from(allNames).sort()) {
    const l = leftMap.get(name)
    const r = rightMap.get(name)
    let delta: TestDelta['delta'] = ''

    if (l === 'fail' && r === 'pass') { delta = 'fixed'; fixes++ }
    else if (l === 'pass' && r === 'fail') { delta = 'REGRESS'; regressions++ }
    else if (!l && r) { delta = 'new' }
    else if (l && !r) { delta = 'removed' }

    tests.push({ name, left: l, right: r, delta })
  }

  const score = fixes * SCORE_FIX + regressions * SCORE_REGRESS
  return { tests, fixes, regressions, score }
}

function refLabel(m: RunManifest | null, fallback: string): string {
  if (!m) return fallback
  if (m.branch && m.commit) return `${m.branch}@${m.commit.slice(0, 7)}`
  if (m.commit) return m.commit.slice(0, 7)
  return fallback
}

// ── Compare View (route: /compare/:left/:right) ──

export default function CompareView({ leftRun, rightRun }: { leftRun: string; rightRun: string }) {
  const [leftManifest, setLeftManifest] = useState<RunManifest | null>(null)
  const [rightManifest, setRightManifest] = useState<RunManifest | null>(null)
  const [loading, setLoading] = useState(true)
  const [sharedTab, setSharedTab] = useState<RunTab>('topology')

  useEffect(() => {
    setLoading(true)
    Promise.all([fetchRunJson(leftRun), fetchRunJson(rightRun)]).then(([l, r]) => {
      setLeftManifest(l)
      setRightManifest(r)
      setLoading(false)
    })
  }, [leftRun, rightRun])

  if (loading) {
    return <div className="empty">Loading compare data...</div>
  }

  const leftLabel = refLabel(leftManifest, leftRun)
  const rightLabel = refLabel(rightManifest, rightRun)

  // Compute diff from tests arrays
  const diff = leftManifest && rightManifest
    ? computeDiff(leftManifest, rightManifest)
    : { tests: [] as TestDelta[], fixes: 0, regressions: 0, score: 0 }

  const leftPass = leftManifest?.pass ?? (leftManifest?.tests ?? []).filter(t => t.status === 'pass').length
  const leftFail = leftManifest?.fail ?? (leftManifest?.tests ?? []).filter(t => t.status === 'fail').length
  const leftTotal = leftManifest?.total ?? (leftManifest?.tests ?? []).length
  const rightPass = rightManifest?.pass ?? (rightManifest?.tests ?? []).filter(t => t.status === 'pass').length
  const rightFail = rightManifest?.fail ?? (rightManifest?.tests ?? []).filter(t => t.status === 'fail').length
  const rightTotal = rightManifest?.total ?? (rightManifest?.tests ?? []).length

  return (
    <div style={{ padding: '1rem', display: 'flex', flexDirection: 'column', height: '100%' }}>
      <h2>Compare: {leftLabel} vs {rightLabel}</h2>

      {/* Summary bar */}
      <div className="compare-summary" style={{ display: 'flex', gap: '2rem', padding: '1rem', background: 'var(--surface)', borderRadius: '8px', marginBottom: '1rem', border: '1px solid var(--border)', flexWrap: 'wrap' }}>
        <div>
          <strong>{leftLabel}:</strong> {leftPass}/{leftTotal} pass, {leftFail} fail
        </div>
        <div>
          <strong>{rightLabel}:</strong> {rightPass}/{rightTotal} pass, {rightFail} fail
        </div>
        {diff.fixes > 0 && <div style={{ color: 'var(--green)' }}>Fixes: {diff.fixes}</div>}
        {diff.regressions > 0 && <div style={{ color: 'var(--red)' }}>Regressions: {diff.regressions}</div>}
        <div>
          Score: <span style={{ color: diff.score >= 0 ? 'var(--green)' : 'var(--red)', fontWeight: 'bold' }}>
            {diff.score >= 0 ? '+' : ''}{diff.score}
          </span>
        </div>
      </div>

      {/* Per-test table */}
      {diff.tests.length > 0 && (
        <div className="tbl-wrap" style={{ marginBottom: '1rem' }}>
          <table>
            <thead>
              <tr>
                <th>Test</th>
                <th>{leftLabel}</th>
                <th>{rightLabel}</th>
                <th>Delta</th>
              </tr>
            </thead>
            <tbody>
              {diff.tests.map(({ name, left, right, delta }) => {
                let color = ''
                if (delta === 'fixed') color = 'var(--green)'
                else if (delta === 'REGRESS') color = 'var(--red)'

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
      )}

      {/* Phase 4c: Split-screen co-navigation */}
      <h3 style={{ marginTop: '1rem' }}>Side-by-side view</h3>
      <SplitRunView left={leftRun} right={rightRun} sharedTab={sharedTab} onTabChange={setSharedTab} />
    </div>
  )
}

// ── Phase 4c: Split-screen co-navigation ──

function SplitRunView({ left, right, sharedTab, onTabChange }: {
  left: string
  right: string
  sharedTab: RunTab
  onTabChange: (tab: RunTab) => void
}) {
  return (
    <div style={{ display: 'flex', gap: '1rem', flex: 1, minHeight: 0 }}>
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
        <div style={{ padding: '4px 8px', background: 'var(--surface)', borderBottom: '1px solid var(--border)', fontSize: 12, fontWeight: 600 }}>
          {left}
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
          <SplitRunPanel runName={left} activeTab={sharedTab} onTabChange={onTabChange} />
        </div>
      </div>
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
        <div style={{ padding: '4px 8px', background: 'var(--surface)', borderBottom: '1px solid var(--border)', fontSize: 12, fontWeight: 600 }}>
          {right}
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
          <SplitRunPanel runName={right} activeTab={sharedTab} onTabChange={onTabChange} />
        </div>
      </div>
    </div>
  )
}

function SplitRunPanel({ runName, activeTab, onTabChange }: {
  runName: string
  activeTab: RunTab
  onTabChange: (tab: RunTab) => void
}) {
  const [state, setState] = useState<any>(null)
  const [events, setEvents] = useState<any[]>([])
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [results, setResults] = useState<any>(null)

  useEffect(() => {
    let dead = false
    Promise.all([
      fetchState(runName),
      fetchEvents(runName),
      fetchLogs(runName),
      fetchResults(runName),
    ]).then(([s, e, l, r]) => {
      if (dead) return
      setState(s)
      setEvents(e ?? [])
      setLogs(l)
      setResults(r)
    })
    return () => { dead = true }
  }, [runName])

  const run: RunInfo = { name: runName, label: null, status: null, group: null }

  return (
    <RunView
      run={run}
      state={state}
      events={events}
      logs={logs}
      results={results}
      activeTab={activeTab}
      onTabChange={onTabChange}
    />
  )
}

// ── Shared helpers ──

function statusBadge(status?: string) {
  if (!status) return <span style={{ color: 'var(--text-muted)' }}>&#8212;</span>
  const colors: Record<string, string> = {
    pass: 'var(--green)',
    fail: 'var(--red)',
    ignored: 'var(--text-muted)',
  }
  return <span style={{ color: colors[status] || 'inherit' }}>{status.toUpperCase()}</span>
}
