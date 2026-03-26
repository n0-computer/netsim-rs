import { useCallback, useEffect, useMemo, useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import type { CombinedResults } from './types'
import { fetchRuns, fetchCombinedResults } from './api'
import type { RunInfo } from './api'
import RunSelector, { selectionPath } from './components/RunSelector'
import type { Selection } from './components/RunSelector'
import PerfTab from './components/PerfTab'
import { simLabel } from './utils'

type BatchTab = 'sims' | 'perf'

export default function BatchPage() {
  const location = useLocation()
  const navigate = useNavigate()

  const batchName = location.pathname.startsWith('/group/')
    ? location.pathname.slice('/group/'.length)
    : location.pathname.slice('/batch/'.length)
  const [tab, setTab] = useState<BatchTab>('sims')

  // Run list (for the dropdown)
  const [runs, setRuns] = useState<RunInfo[]>([])
  const [combinedResults, setCombinedResults] = useState<CombinedResults | null>(null)

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

  // ── Load combined results ──

  useEffect(() => {
    if (!batchName) {
      setCombinedResults(null)
      return
    }

    let dead = false
    fetchCombinedResults(batchName).then((results) => {
      if (dead) return
      setCombinedResults(results)
    })

    return () => { dead = true }
  }, [batchName])

  // ── Derived ──

  const selection: Selection | null = batchName ? { kind: 'group', name: batchName } : null
  const groupRuns = useMemo(
    () => runs.filter((r) => r.group === batchName),
    [runs, batchName],
  )

  const availableTabs = useMemo<BatchTab[]>(
    () => ['sims', ...(combinedResults ? (['perf'] as BatchTab[]) : [])],
    [combinedResults],
  )

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

  return (
    <div className="app">
      <div className="topbar">
        <h1><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>patchbay</a></h1>
        <RunSelector runs={runs} value={selection} onChange={handleSelectionChange} />
      </div>

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
        {tab === 'sims' && (
          <div className="sims-list">
            <h2>{batchName}</h2>
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
