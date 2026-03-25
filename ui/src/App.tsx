import { useCallback, useEffect, useRef, useState } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import type {
  Firewall,
  LabEvent,
  LabState,
  LinkCondition,
  Nat,
  NatV6Mode,
  RouterState,
  DeviceState,
  IfaceState,
} from './devtools-types'
import type { CombinedResults, SimResults } from './types'
import {
  fetchRuns,
  fetchState,
  fetchEvents,
  subscribeEvents,
  fetchLogs,
  fetchResults,
  fetchCombinedResults,
  runFilesBase,
} from './api'
import type { RunInfo, LogEntry } from './api'
import PerfTab from './components/PerfTab'
import RunView from './components/RunView'
import type { RunTab } from './components/RunView'
import CompareView from './components/CompareView'

type Tab = 'topology' | 'logs' | 'timeline' | 'perf' | 'metrics' | 'sims'

// ── Selection model ────────────────────────────────────────────────

type Selection =
  | { kind: 'run'; name: string }
  | { kind: 'batch'; name: string }

function selectionKey(s: Selection | null): string {
  if (!s) return ''
  return s.kind === 'batch' ? `batch:${s.name}` : s.name
}

function selectionPath(s: Selection | null): string {
  if (!s) return '/'
  return s.kind === 'batch' ? `/batch/${s.name}` : `/run/${s.name}`
}

// ── Batch grouping ─────────────────────────────────────────────────

interface BatchGroup {
  batch: string
  runs: RunInfo[]
}

function groupByBatch(runs: RunInfo[]): { groups: BatchGroup[]; ungrouped: RunInfo[] } {
  const grouped = new Map<string, RunInfo[]>()
  const ungrouped: RunInfo[] = []
  for (const r of runs) {
    if (r.batch) {
      let list = grouped.get(r.batch)
      if (!list) {
        list = []
        grouped.set(r.batch, list)
      }
      list.push(r)
    } else {
      ungrouped.push(r)
    }
  }
  const groups: BatchGroup[] = []
  for (const [batch, groupRuns] of grouped) {
    groups.push({ batch, runs: groupRuns })
  }
  return { groups, ungrouped }
}

/** Short display label for a run within a batch group. */
function simLabel(run: RunInfo): string {
  if (run.batch && run.name.startsWith(run.batch + '/')) {
    return run.label ?? run.name.slice(run.batch.length + 1)
  }
  return run.label ?? run.name
}

// ── State reducer (from DevtoolsApp) ──────────────────────────────

function applyEvent(state: LabState, event: LabEvent): LabState {
  const next = { ...state, opid: event.opid }
  const kind = event.kind

  if (kind === 'router_added') {
    const name = event.name as string
    const routerState: RouterState = {
      ns: event.ns as string,
      region: (event.region as string | null) ?? null,
      nat: event.nat as Nat,
      nat_v6: event.nat_v6 as NatV6Mode,
      firewall: event.firewall as Firewall,
      ip_support: event.ip_support as RouterState['ip_support'],
      mtu: (event.mtu as number | null) ?? null,
      upstream: (event.upstream as string | null) ?? null,
      uplink_ip: (event.uplink_ip as string | null) ?? null,
      uplink_ip_v6: (event.uplink_ip_v6 as string | null) ?? null,
      downstream_cidr: (event.downstream_cidr as string | null) ?? null,
      downstream_gw: (event.downstream_gw as string | null) ?? null,
      downstream_cidr_v6: (event.downstream_cidr_v6 as string | null) ?? null,
      downstream_gw_v6: (event.downstream_gw_v6 as string | null) ?? null,
      downstream_bridge: event.downstream_bridge as string,
      downlink_condition: (event.downlink_condition as LinkCondition | null) ?? null,
      devices: (event.devices as string[]) ?? [],
      counters: (event.counters as Record<string, RouterState['counters'][string]>) ?? {},
    }
    next.routers = { ...next.routers, [name]: routerState }
  } else if (kind === 'router_removed') {
    const { [event.name as string]: _, ...rest } = next.routers
    next.routers = rest
  } else if (kind === 'device_added') {
    const name = event.name as string
    const deviceState: DeviceState = {
      ns: event.ns as string,
      default_via: event.default_via as string,
      mtu: (event.mtu as number | null) ?? null,
      interfaces: (event.interfaces as IfaceState[]) ?? [],
      counters: (event.counters as Record<string, DeviceState['counters'][string]>) ?? {},
    }
    for (const iface of deviceState.interfaces) {
      const router = next.routers[iface.router]
      if (router && !router.devices.includes(name)) {
        next.routers = {
          ...next.routers,
          [iface.router]: { ...router, devices: [...router.devices, name] },
        }
      }
    }
    next.devices = { ...next.devices, [name]: deviceState }
  } else if (kind === 'device_removed') {
    const name = event.name as string
    const dev = next.devices[name]
    if (dev) {
      for (const iface of dev.interfaces) {
        const router = next.routers[iface.router]
        if (router) {
          next.routers = {
            ...next.routers,
            [iface.router]: { ...router, devices: router.devices.filter((d) => d !== name) },
          }
        }
      }
    }
    const { [name]: _, ...rest } = next.devices
    next.devices = rest
  } else if (kind === 'nat_changed') {
    const router = next.routers[event.router as string]
    if (router) {
      next.routers = { ...next.routers, [event.router as string]: { ...router, nat: event.nat as Nat } }
    }
  } else if (kind === 'firewall_changed') {
    const router = next.routers[event.router as string]
    if (router) {
      next.routers = { ...next.routers, [event.router as string]: { ...router, firewall: event.firewall as Firewall } }
    }
  }

  return next
}

// ── Unified App ────────────────────────────────────────────────────

export default function App({ mode }: { mode: 'run' | 'batch' | 'compare' }) {
  const location = useLocation()
  const navigate = useNavigate()

  // Derive selection from the URL path.
  // Route is /run/*, /batch/*, or /compare/* so everything after the prefix is the name.
  const prefixes: Record<string, string> = { run: '/run/', batch: '/batch/', compare: '/compare/' }
  const prefixLen = prefixes[mode]?.length ?? '/run/'.length
  const nameFromUrl = location.pathname.slice(prefixLen)
  const effectiveKind = mode === 'compare' ? 'batch' : mode
  const selection: Selection | null = nameFromUrl
    ? { kind: effectiveKind === 'batch' ? 'batch' : 'run', name: nameFromUrl }
    : null

  const selectedRun = selection?.kind === 'run' ? selection.name : null
  const selectedBatch = selection?.kind === 'batch' ? selection.name : null

  const [tab, setTab] = useState<Tab>(mode === 'batch' || mode === 'compare' ? 'sims' : 'topology')

  // Run list (for the dropdown)
  const [runs, setRuns] = useState<RunInfo[]>([])

  // Lab state and events
  const [labState, setLabState] = useState<LabState | null>(null)
  const [labEvents, setLabEvents] = useState<LabEvent[]>([])
  const esRef = useRef<EventSource | null>(null)
  const lastOpidRef = useRef<number>(0)

  // Log files
  const [logList, setLogList] = useState<LogEntry[]>([])

  // Perf results
  const [simResults, setSimResults] = useState<SimResults | null>(null)
  const [combinedResults, setCombinedResults] = useState<CombinedResults | null>(null)

  // Compare manifest detection for batch view
  const [hasCompareManifest, setHasCompareManifest] = useState(false)

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

  // ── Load run data when an individual sim is selected ──

  useEffect(() => {
    if (!selectedRun) {
      setLabState(null)
      setLabEvents([])
      setLogList([])
      setSimResults(null)
      return
    }

    let dead = false
    Promise.all([
      fetchState(selectedRun),
      fetchEvents(selectedRun),
      fetchLogs(selectedRun),
      fetchResults(selectedRun),
    ]).then(([state, events, logs, results]) => {
      if (dead) return
      if (state) setLabState(state)
      setLabEvents(events)
      lastOpidRef.current = events.length ? Math.max(...events.map((e) => e.opid ?? 0)) : 0
      setLogList(logs)
      setSimResults(results)
    })

    return () => { dead = true }
  }, [selectedRun])

  // ── Load combined results when a batch is selected ──

  useEffect(() => {
    if (!selectedBatch) {
      setCombinedResults(null)
      return
    }

    let dead = false
    fetchCombinedResults(selectedBatch).then((results) => {
      if (dead) return
      setCombinedResults(results)
    })

    return () => { dead = true }
  }, [selectedBatch])

  // ── Check for compare manifest (summary.json) in batch ──

  useEffect(() => {
    if (!selectedBatch) {
      setHasCompareManifest(false)
      return
    }

    let dead = false
    fetch(`${runFilesBase(selectedBatch)}summary.json`, { method: 'HEAD' })
      .then((res) => {
        if (!dead) setHasCompareManifest(res.ok)
      })
      .catch(() => {
        if (!dead) setHasCompareManifest(false)
      })

    return () => { dead = true }
  }, [selectedBatch])

  // ── SSE for live updates (only when run is "running") ──

  useEffect(() => {
    if (!selectedRun) return
    const runInfo = runs.find((r) => r.name === selectedRun)
    if (runInfo?.status !== 'running') return

    const es = subscribeEvents(selectedRun, lastOpidRef.current, (event) => {
      setLabState((prev) => (prev ? applyEvent(prev, event) : prev))
      setLabEvents((prev) => [...prev.slice(-999), event])
      if (event.opid != null) lastOpidRef.current = event.opid
    })
    esRef.current = es
    return () => {
      es.close()
      esRef.current = null
    }
  }, [selectedRun, runs])

  // Close SSE when tab becomes hidden, reconnect when visible.
  useEffect(() => {
    const onVisibility = () => {
      if (document.hidden) {
        esRef.current?.close()
        esRef.current = null
      }
    }
    document.addEventListener('visibilitychange', onVisibility)
    window.addEventListener('beforeunload', () => esRef.current?.close())
    return () => document.removeEventListener('visibilitychange', onVisibility)
  }, [])

  // ── Derived ──

  const isSimView = selection?.kind === 'run'
  const isBatchView = selection?.kind === 'batch'

  // Runs belonging to the current batch
  const batchRuns = isBatchView
    ? runs.filter((r) => r.batch === selectedBatch)
    : []

  const hasMetricsLogs = logList.some(l => l.kind === 'metrics')
  const availableTabs: Tab[] = isSimView
    ? ['topology', 'logs', 'timeline', ...(simResults ? (['perf'] as Tab[]) : []), ...(hasMetricsLogs ? (['metrics'] as Tab[]) : [])]
    : isBatchView
      ? ['sims', ...(combinedResults ? (['perf'] as Tab[]) : [])]
      : []

  // When available tabs change, ensure current tab is still valid.
  useEffect(() => {
    if (availableTabs.length > 0 && !availableTabs.includes(tab)) {
      setTab(availableTabs[0])
    }
  }, [availableTabs, tab])

  // Resolved run info for the selected run
  const selectedRunInfo = isSimView ? runs.find((r) => r.name === selectedRun) ?? null : null

  // Group runs for the selector
  const { groups, ungrouped } = groupByBatch(runs)

  // ── Render ──

  return (
    <div className="app">
      <div className="topbar">
        <h1><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>patchbay</a></h1>
        <select
          value={selectionKey(selection)}
          onChange={(e) => {
            const val = e.target.value
            if (!val) {
              navigate('/')
              return
            }
            if (val.startsWith('batch:')) {
              navigate(`/batch/${val.slice(6)}`)
            } else {
              navigate(`/run/${val}`)
            }
          }}
        >
          <option value="">select run</option>
          {groups.map((g) => (
            <optgroup key={g.batch} label={g.batch}>
              {g.runs.length > 1 && (
                <option value={`batch:${g.batch}`}>
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
        {isSimView && selectedRunInfo && (
          <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>
            {selectedRunInfo.status ?? ''}
          </span>
        )}
        {labState && (
          <span style={{ color: 'var(--text-muted)', fontSize: 11 }}>
            opid: {labState.opid}
          </span>
        )}
      </div>

      {isSimView && selectedRun && (
        <RunView
          run={selectedRunInfo ?? { name: selectedRun, label: null, status: null, batch: null }}
          state={labState}
          events={labEvents}
          logs={logList}
          results={simResults}
          activeTab={tab as RunTab}
          onTabChange={(t) => setTab(t)}
        />
      )}

      {isBatchView && hasCompareManifest && (
        <CompareView batchName={selectedBatch!} />
      )}

      {isBatchView && !hasCompareManifest && (
        <>
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
                <h2>{selectedBatch}</h2>
                {batchRuns.length === 0 && <div className="empty">No sims found.</div>}
                {batchRuns.map((r) => (
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

            {tab === 'perf' && <PerfTab results={null} combined={combinedResults} onSimSelect={(sim) => navigate(`/run/${sim}`)} />}
          </div>
        </>
      )}
    </div>
  )
}
