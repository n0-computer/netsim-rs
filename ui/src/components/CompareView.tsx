import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import type { LabEvent, LabState } from '../devtools-types'
import type { SimResults } from '../types'
import { fetchRunJson, fetchRuns, fetchState, fetchEvents, fetchLogs, fetchResults } from '../api'
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
  /** Left-side run path for this test (if available). */
  leftDir?: string
  /** Right-side run path for this test (if available). */
  rightDir?: string
}

function computeDiff(left: RunManifest, right: RunManifest) {
  const leftTests = left.tests ?? []
  const rightTests = right.tests ?? []
  const leftMap = new Map(leftTests.map(t => [t.name, t.status]))
  const rightMap = new Map(rightTests.map(t => [t.name, t.status]))
  const leftDirMap = new Map(leftTests.filter((t): t is typeof t & { dir: string } => !!t.dir).map(t => [t.name, t.dir]))
  const rightDirMap = new Map(rightTests.filter((t): t is typeof t & { dir: string } => !!t.dir).map(t => [t.name, t.dir]))

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

    const leftDir = leftDirMap.get(name)
    const rightDir = rightDirMap.get(name)
    tests.push({ name, left: l, right: r, delta, leftDir, rightDir })
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

/** Extract the last path segment as a short display name. */
function shortName(runPath: string): string {
  const parts = runPath.split('/')
  return parts[parts.length - 1] || runPath
}

/** Check if this is a group compare (has tests) vs individual run compare. */
function isGroupCompare(left: RunManifest | null, right: RunManifest | null): boolean {
  const leftTests = left?.tests ?? []
  const rightTests = right?.tests ?? []
  return leftTests.length > 0 || rightTests.length > 0
}

/** Extract the group (first path segment) from a run path like "run-20260326_123338/project/test". */
function extractGroup(runPath: string): string {
  return runPath.split('/')[0] || runPath
}

/** Build the parent group compare URL from two individual run paths. */
function groupCompareUrl(leftRun: string, rightRun: string): string {
  const leftGroup = extractGroup(leftRun)
  const rightGroup = extractGroup(rightRun)
  return `/compare/${encodeURIComponent(leftGroup)}/${encodeURIComponent(rightGroup)}`
}

/** Build a synthetic manifest from a group's child runs. */
function manifestFromGroup(groupName: string, runs: RunInfo[]): RunManifest {
  const children = runs.filter(r => r.group === groupName)
  const tests = children.map(r => {
    // Strip group prefix to get the test name (e.g. "testdir-2/patchbay/holepunch" → "patchbay/holepunch")
    const testName = r.name.startsWith(groupName + '/') ? r.name.slice(groupName.length + 1) : r.name
    const status = r.status === 'success' ? 'pass' : r.status === 'error' ? 'fail' : (r.status ?? 'pass')
    return { name: testName, status, dir: r.name }
  })
  const pass = tests.filter(t => t.status === 'pass').length
  const fail = tests.filter(t => t.status === 'fail').length
  // Use the group-level manifest for context if available
  const groupManifest = children[0]?.manifest
  return {
    kind: groupManifest?.kind ?? 'test',
    project: groupManifest?.project ?? null,
    branch: groupManifest?.branch ?? null,
    commit: groupManifest?.commit ?? null,
    pass, fail, total: tests.length,
    tests,
    outcome: fail > 0 ? 'fail' : 'pass',
  } as RunManifest
}

// ── Compare View (route: /compare/:left/:right) ──

export default function CompareView({ leftRun, rightRun }: { leftRun: string; rightRun: string }) {
  const navigate = useNavigate()
  const [leftManifest, setLeftManifest] = useState<RunManifest | null>(null)
  const [rightManifest, setRightManifest] = useState<RunManifest | null>(null)
  const [loading, setLoading] = useState(true)
  const [sharedTab, setSharedTab] = useState<RunTab>('logs')

  useEffect(() => {
    let dead = false
    setLoading(true)
    // Try fetching as individual runs first.
    Promise.all([fetchRunJson(leftRun), fetchRunJson(rightRun)]).then(async ([l, r]) => {
      if (dead) return
      // If both are null, these might be group names — build manifests from children.
      if (!l || !r) {
        const allRuns = await fetchRuns()
        if (dead) return
        if (!l) l = manifestFromGroup(leftRun, allRuns)
        if (!r) r = manifestFromGroup(rightRun, allRuns)
      }
      setLeftManifest(l)
      setRightManifest(r)
      setLoading(false)
    })
    return () => { dead = true }
  }, [leftRun, rightRun])

  if (loading) {
    return <div className="empty">Loading compare data...</div>
  }

  const leftLabel = refLabel(leftManifest, leftRun)
  const rightLabel = refLabel(rightManifest, rightRun)
  const isGroup = isGroupCompare(leftManifest, rightManifest)

  // Compute diff from tests arrays
  const diff = leftManifest && rightManifest
    ? computeDiff(leftManifest, rightManifest)
    : { tests: [] as TestDelta[], fixes: 0, regressions: 0, score: 0 }

  const leftPass = leftManifest?.pass ?? (leftManifest?.tests ?? []).filter(t => t.status === 'pass').length
  const leftTotal = leftManifest?.total ?? (leftManifest?.tests ?? []).length
  const rightPass = rightManifest?.pass ?? (rightManifest?.tests ?? []).filter(t => t.status === 'pass').length
  const rightTotal = rightManifest?.total ?? (rightManifest?.tests ?? []).length
  const leftOutcome = leftManifest?.test_outcome ?? leftManifest?.outcome ?? null
  const rightOutcome = rightManifest?.test_outcome ?? rightManifest?.outcome ?? null

  const handleTestClick = (leftDir: string, rightDir: string) => {
    navigate(`/compare/${encodeURIComponent(leftDir)}/${encodeURIComponent(rightDir)}`)
  }

  if (isGroup) {
    // Group compare: render ONLY the diff table, no SplitRunView.
    return (
      <div style={{ padding: '1rem', display: 'flex', flexDirection: 'column', height: '100%' }}>
        <h2 style={{ margin: '0 0 0.5rem 0' }}>
          Compare: {leftLabel} vs {rightLabel} — {leftPass}/{leftTotal} → {rightPass}/{rightTotal}
          {diff.regressions > 0 && <span style={{ color: 'var(--red)' }}> ({diff.regressions} regression{diff.regressions > 1 ? 's' : ''})</span>}
          {diff.fixes > 0 && <span style={{ color: 'var(--green)' }}> ({diff.fixes} fix{diff.fixes > 1 ? 'es' : ''})</span>}
        </h2>

        {/* Summary bar */}
        <div className="compare-summary" style={{ display: 'flex', gap: '2rem', padding: '0.5rem 1rem', background: 'var(--surface)', borderRadius: '8px', marginBottom: '1rem', border: '1px solid var(--border)', alignItems: 'center' }}>
          <div>
            Score: <span style={{ color: diff.score >= 0 ? 'var(--green)' : 'var(--red)', fontWeight: 'bold' }}>
              {diff.score >= 0 ? '+' : ''}{diff.score}
            </span>
          </div>
        </div>

        {/* Per-test table */}
        {diff.tests.length > 0 && (
          <div className="tbl-wrap" style={{ marginBottom: '1rem', flex: 1, overflow: 'auto' }}>
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
                {diff.tests.map(({ name, left, right, delta, leftDir, rightDir }) => {
                  let color = ''
                  if (delta === 'fixed') color = 'var(--green)'
                  else if (delta === 'REGRESS') color = 'var(--red)'

                  const canClick = !!(leftDir && rightDir)
                  return (
                    <tr key={name}>
                      <td>
                        {canClick ? (
                          <code
                            style={{ cursor: 'pointer', textDecoration: 'underline', textDecorationColor: 'var(--text-muted)' }}
                            onClick={() => handleTestClick(leftDir, rightDir)}
                            title={`Compare ${name} side-by-side`}
                          >
                            {name}
                          </code>
                        ) : (
                          <code style={{ color: 'var(--text-muted)' }} title="No test output available">
                            {name}
                          </code>
                        )}
                      </td>
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
      </div>
    )
  }

  // Individual run compare: render ONLY the SplitRunView with back-to-group link.
  return (
    <div style={{ padding: '1rem', display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div style={{ margin: '0 0 1rem 0' }}>
        <h2 style={{ margin: '0 0 0.5rem 0' }}>
          Compare: {shortName(leftRun)} (left) vs {shortName(rightRun)} (right)
        </h2>
        <Link to={groupCompareUrl(leftRun, rightRun)} style={{ fontSize: 13, color: 'var(--accent, #4a9eff)' }}>
          &#x21A9; Back to group compare
        </Link>
      </div>
      <SplitRunView left={leftRun} right={rightRun} sharedTab={sharedTab} onTabChange={setSharedTab} />
    </div>
  )
}

// ── Shared controls state ──

interface SharedControls {
  logFilter: string
  logLevels: Set<string>
  metricsFilter: string
}

const ALL_LEVELS = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'] as const

function SharedControlsBar({ controls, onChange, activeTab }: {
  controls: SharedControls
  onChange: (updates: Partial<SharedControls>) => void
  activeTab: RunTab
}) {
  const toggleLevel = useCallback((level: string) => {
    const next = new Set(controls.logLevels)
    if (next.has(level)) next.delete(level)
    else next.add(level)
    onChange({ logLevels: next })
  }, [controls.logLevels, onChange])

  if (activeTab === 'logs') {
    return (
      <div className="logs-toolbar" style={{ marginBottom: '0.5rem', flexShrink: 0 }}>
        <span style={{ fontWeight: 600, fontSize: 12 }}>Shared:</span>
        {ALL_LEVELS.map((level) => (
          <span
            key={level}
            className={`level-toggle level-${level} ${controls.logLevels.has(level) ? 'on' : 'off'}`}
            onClick={() => toggleLevel(level)}
            style={{ cursor: 'pointer' }}
          >
            {level}
          </span>
        ))}
        <input
          type="search"
          placeholder="filter logs..."
          value={controls.logFilter}
          onChange={(e) => onChange({ logFilter: e.target.value })}
          style={{ marginLeft: 'auto', minWidth: 180 }}
        />
      </div>
    )
  }

  if (activeTab === 'metrics') {
    return (
      <div className="logs-toolbar" style={{ marginBottom: '0.5rem', flexShrink: 0 }}>
        <span style={{ fontWeight: 600, fontSize: 12 }}>Shared:</span>
        <input
          type="search"
          placeholder="filter metrics..."
          value={controls.metricsFilter}
          onChange={(e) => onChange({ metricsFilter: e.target.value })}
          style={{ minWidth: 180 }}
        />
      </div>
    )
  }

  return null
}

// ── Split-screen co-navigation ──

function SplitRunView({ left, right, sharedTab, onTabChange }: {
  left: string
  right: string
  sharedTab: RunTab
  onTabChange: (tab: RunTab) => void
}) {
  const [sharedControls, setSharedControls] = useState<SharedControls>({
    logFilter: '',
    logLevels: new Set(ALL_LEVELS),
    metricsFilter: '',
  })

  const handleControlsChange = useCallback((updates: Partial<SharedControls>) => {
    setSharedControls(prev => ({ ...prev, ...updates }))
  }, [])

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <SharedControlsBar controls={sharedControls} onChange={handleControlsChange} activeTab={sharedTab} />
      <div style={{ display: 'flex', gap: '1rem', flex: 1, minHeight: 0 }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
          <div style={{ padding: '4px 8px', background: 'var(--surface)', borderBottom: '1px solid var(--border)', fontSize: 12, fontWeight: 600 }}>
            {left}
          </div>
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <SplitRunPanel runName={left} activeTab={sharedTab} onTabChange={onTabChange} sharedControls={sharedControls} />
          </div>
        </div>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
          <div style={{ padding: '4px 8px', background: 'var(--surface)', borderBottom: '1px solid var(--border)', fontSize: 12, fontWeight: 600 }}>
            {right}
          </div>
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <SplitRunPanel runName={right} activeTab={sharedTab} onTabChange={onTabChange} sharedControls={sharedControls} />
          </div>
        </div>
      </div>
    </div>
  )
}

function SplitRunPanel({ runName, activeTab, onTabChange, sharedControls }: {
  runName: string
  activeTab: RunTab
  onTabChange: (tab: RunTab) => void
  sharedControls: SharedControls
}) {
  const [state, setState] = useState<LabState | null>(null)
  const [events, setEvents] = useState<LabEvent[]>([])
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [results, setResults] = useState<SimResults | null>(null)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    let dead = false
    setLoaded(false)
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
      setLoaded(true)
    })
    return () => { dead = true }
  }, [runName])

  const run: RunInfo = { name: runName, label: null, status: null, group: null }

  const externalControls = useMemo(() => ({
    logFilter: sharedControls.logFilter,
    logLevels: sharedControls.logLevels,
    metricsFilter: sharedControls.metricsFilter,
  }), [sharedControls.logFilter, sharedControls.logLevels, sharedControls.metricsFilter])

  return (
    <RunView
      run={run}
      state={state}
      events={events}
      logs={logs}
      results={results}
      activeTab={activeTab}
      onTabChange={onTabChange}
      externalControls={externalControls}
      loaded={loaded}
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

