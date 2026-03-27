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
import type { SimResults } from './types'
import {
  fetchRuns,
  fetchState,
  fetchEvents,
  fetchLogs,
  fetchResults,
  subscribeEvents,
} from './api'
import type { RunInfo, LogEntry } from './api'
import RunSelector, { selectionPath } from './components/RunSelector'
import type { Selection } from './components/RunSelector'
import RunView from './components/RunView'
import type { RunTab } from './components/RunView'

// ── State reducer ──────────────────────────────────────────────────

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

// ── RunPage ────────────────────────────────────────────────────────

export default function RunPage() {
  const location = useLocation()
  const navigate = useNavigate()

  const runName = location.pathname.slice('/run/'.length)
  const [tab, setTab] = useState<RunTab>('topology')

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

  // ── Load run data ──

  useEffect(() => {
    if (!runName) {
      setLabState(null)
      setLabEvents([])
      setLogList([])
      setSimResults(null)
      return
    }

    let dead = false
    Promise.all([
      fetchState(runName),
      fetchEvents(runName),
      fetchLogs(runName),
      fetchResults(runName),
    ]).then(([state, events, logs, results]) => {
      if (dead) return
      if (state) setLabState(state)
      setLabEvents(events)
      lastOpidRef.current = events.length ? Math.max(...events.map((e) => e.opid ?? 0)) : 0
      setLogList(logs)
      setSimResults(results)
    })

    return () => { dead = true }
  }, [runName])

  // ── SSE for live updates (only when run is "running") ──

  useEffect(() => {
    if (!runName) return
    const runInfo = runs.find((r) => r.name === runName)
    if (runInfo?.status !== 'running') return

    const es = subscribeEvents(runName, lastOpidRef.current, (event) => {
      setLabState((prev) => (prev ? applyEvent(prev, event) : prev))
      setLabEvents((prev) => [...prev.slice(-999), event])
      if (event.opid != null) lastOpidRef.current = event.opid
    })
    esRef.current = es
    return () => {
      es.close()
      esRef.current = null
    }
  }, [runName, runs])

  // Close SSE when tab becomes hidden.
  useEffect(() => {
    const onVisibility = () => {
      if (document.hidden) {
        esRef.current?.close()
        esRef.current = null
      }
    }
    const onUnload = () => esRef.current?.close()
    document.addEventListener('visibilitychange', onVisibility)
    window.addEventListener('beforeunload', onUnload)
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      window.removeEventListener('beforeunload', onUnload)
    }
  }, [])

  // ── Derived ──

  const selection: Selection | null = runName ? { kind: 'run', name: runName } : null
  const selectedRunInfo = runs.find((r) => r.name === runName) ?? null

  const handleSelectionChange = useCallback((sel: Selection | null) => {
    navigate(selectionPath(sel))
  }, [navigate])

  // ── Render ──

  return (
    <div className="app">
      <div className="topbar">
        <h1><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>patchbay</a></h1>
        <RunSelector runs={runs} value={selection} onChange={handleSelectionChange} />
        {selectedRunInfo && (
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

      {runName && (
        <RunView
          run={selectedRunInfo ?? { name: runName, label: null, status: null, group: null }}
          state={labState}
          events={labEvents}
          logs={logList}
          results={simResults}
          activeTab={tab}
          onTabChange={setTab}
        />
      )}
    </div>
  )
}
