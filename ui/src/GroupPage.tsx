import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import type { CombinedResults } from './types'
import { fetchRuns, fetchCombinedResults, fetchRunJson } from './api'
import type { RunInfo, RunManifest, TestResult } from './api'
import RunSelector, { selectionPath } from './components/RunSelector'
import type { Selection } from './components/RunSelector'
import PerfTab from './components/PerfTab'
import { simLabel } from './utils'

type GroupTab = 'tests' | 'sims' | 'perf'

export default function GroupPage() {
  const location = useLocation()
  const navigate = useNavigate()

  const groupName = location.pathname.slice('/group/'.length)
  const [tab, setTab] = useState<GroupTab>('sims')

  // Run list (for the dropdown)
  const [runs, setRuns] = useState<RunInfo[]>([])
  const [combinedResults, setCombinedResults] = useState<CombinedResults | null>(null)
  const [manifest, setManifest] = useState<RunManifest | null>(null)

  // ── Poll runs list ──

  const refreshRuns = useCallback(async () => {
    const r = await fetchRuns()
    setRuns(r)
  }, [])

  useEffect(() => {
    refreshRuns()
    const id = setInterval(refreshRuns, 5_000)
    return () => clearInterval(id)
  }, [refreshRuns])

  // ── Load manifest ──

  useEffect(() => {
    if (!groupName) {
      setManifest(null)
      return
    }
    let dead = false
    fetchRunJson(groupName).then((m) => {
      if (!dead) setManifest(m)
    })
    return () => { dead = true }
  }, [groupName])

  // ── Load combined results ──

  useEffect(() => {
    if (!groupName) {
      setCombinedResults(null)
      return
    }

    let dead = false
    fetchCombinedResults(groupName).then((results) => {
      if (dead) return
      setCombinedResults(results)
    })

    return () => { dead = true }
  }, [groupName])

  // ── Derived ──

  const selection: Selection | null = groupName ? { kind: 'group', name: groupName } : null
  const groupRuns = useMemo(
    () => runs.filter((r) => r.group === groupName),
    [runs, groupName],
  )

  const isTestGroup = manifest?.kind === 'test'

  const availableTabs = useMemo<GroupTab[]>(() => {
    const tabs: GroupTab[] = []
    if (isTestGroup) {
      tabs.push('tests')
    } else {
      tabs.push('sims')
    }
    if (combinedResults) tabs.push('perf')
    return tabs
  }, [isTestGroup, combinedResults])

  // Ensure current tab is still valid when available tabs change.
  useEffect(() => {
    if (availableTabs.length > 0 && !availableTabs.includes(tab)) {
      setTab(availableTabs[0])
    }
  }, [availableTabs, tab])

  const handleSelectionChange = useCallback((sel: Selection | null) => {
    navigate(selectionPath(sel))
  }, [navigate])

  // ── Render ──

  const outcome = manifest?.outcome ?? manifest?.test_outcome
  const outcomeColor = outcome === 'pass' || outcome === 'success' ? 'var(--green)' : outcome === 'fail' || outcome === 'failure' ? 'var(--red)' : 'var(--text-muted)'
  const commitShort = manifest?.commit?.slice(0, 7)
  const tests: TestResult[] = manifest?.tests ?? []

  return (
    <div className="app">
      <div className="topbar">
        <h1><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>patchbay</a></h1>
        <RunSelector runs={runs} value={selection} onChange={handleSelectionChange} />
      </div>

      {/* Manifest header for test groups */}
      {manifest && isTestGroup && (
        <div style={{ padding: '0.75rem 1rem', background: 'var(--surface)', borderBottom: '1px solid var(--border)', display: 'flex', gap: '1rem', alignItems: 'center', flexWrap: 'wrap' }}>
          {manifest.project && <span style={{ fontWeight: 600 }}>{manifest.project}</span>}
          {manifest.branch && commitShort && (
            <code style={{ fontSize: 12 }}>{manifest.branch}@{commitShort}</code>
          )}
          {manifest.pr != null && manifest.pr_url && (
            <a href={manifest.pr_url} target="_blank" rel="noopener noreferrer" style={{ fontSize: 12, color: 'var(--accent, #4a9eff)' }}>
              #{manifest.pr}
            </a>
          )}
          {manifest.pr != null && !manifest.pr_url && (
            <span style={{ fontSize: 12, color: 'var(--text-muted)' }}>#{manifest.pr}</span>
          )}
          {manifest.title && <span style={{ fontSize: 12, color: 'var(--text-muted)' }}>{manifest.title}</span>}
          {outcome && (
            <span style={{ fontSize: 11, fontWeight: 600, textTransform: 'uppercase', color: outcomeColor }}>
              {outcome}
            </span>
          )}
          {manifest.pass != null && manifest.total != null && (
            <span style={{ fontSize: 12 }}>{manifest.pass}/{manifest.total} pass</span>
          )}
          {manifest.fail != null && manifest.fail > 0 && (
            <span style={{ fontSize: 12, color: 'var(--red)' }}>{manifest.fail} fail</span>
          )}
        </div>
      )}

      <div className="tabs">
        {availableTabs.map((t) => (
          <button
            key={t}
            className={`tab-btn${tab === t ? ' active' : ''}`}
            onClick={() => setTab(t)}
          >
            {t}
          </button>
        ))}
      </div>

      <div className="tab-content" style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {tab === 'tests' && (
          <div style={{ flex: 1, overflow: 'auto', padding: '1rem' }}>
            <h2>{groupName}</h2>
            {tests.length === 0 && <div className="empty">No test results found.</div>}
            {tests.length > 0 && (
              <div className="tbl-wrap">
                <table>
                  <thead>
                    <tr>
                      <th>Test</th>
                      <th>Status</th>
                      <th>Duration</th>
                    </tr>
                  </thead>
                  <tbody>
                    {tests.map((t) => {
                      const statusColor = t.status === 'pass' ? 'var(--green)' : t.status === 'fail' ? 'var(--red)' : 'var(--text-muted)'
                      const durationStr = t.duration != null ? `${(t.duration / 1000).toFixed(2)}s` : '\u2014'
                      return (
                        <tr key={t.name}>
                          <td>
                            {t.dir ? (
                              <Link to={`/run/${t.dir}`} style={{ color: 'inherit' }}>
                                <code>{t.name}</code>
                              </Link>
                            ) : (
                              <code style={{ color: 'var(--text-muted)' }}>{t.name}</code>
                            )}
                          </td>
                          <td><span style={{ color: statusColor, fontWeight: 600 }}>{t.status.toUpperCase()}</span></td>
                          <td>{durationStr}</td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        )}

        {tab === 'sims' && (
          <div className="sims-list">
            <h2>{groupName}</h2>
            {groupRuns.length === 0 && <div className="empty">No sims found.</div>}
            {groupRuns.map((r) => (
              <a
                key={r.name}
                href={`/run/${r.name}`}
                className="run-entry"
                onClick={(e) => { e.preventDefault(); navigate(`/run/${r.name}`) }}
              >
                <span className="run-entry-label">{simLabel(r)}</span>
                {r.status && <span className="run-entry-status">{r.status}</span>}
              </a>
            ))}
          </div>
        )}

        {tab === 'perf' && (
          <PerfTab results={null} combined={combinedResults} onSimSelect={(sim) => navigate(`/run/${sim}`)} />
        )}
      </div>
    </div>
  )
}
