import { Fragment, useEffect, useMemo, useState } from 'react'
import { runFilesBase } from '../api'
import type { LogEntry } from '../api'

interface MetricPoint { t: number; m: Record<string, number> }

interface MetricSeries {
  device: string
  key: string
  values: { t: number; v: number }[]
}

function Sparkline({ values }: { values: number[] }) {
  if (values.length < 2) return null
  const w = 80, h = 20
  const min = Math.min(...values)
  const max = Math.max(...values)
  const range = max - min || 1
  const points = values.map((v, i) =>
    `${(i / (values.length - 1)) * w},${h - ((v - min) / range) * h}`
  ).join(' ')
  return (
    <svg width={w} height={h} style={{ verticalAlign: 'middle' }}>
      <polyline points={points} fill="none" stroke="var(--accent)" strokeWidth="1.5" />
    </svg>
  )
}

interface MetricsTabProps {
  run: string
  logs: LogEntry[]
  /** When provided, use this filter instead of internal state. */
  sharedFilter?: string
}

export default function MetricsTab({ run, logs, sharedFilter }: MetricsTabProps) {
  const [series, setSeries] = useState<MetricSeries[]>([])
  const hasSharedFilter = sharedFilter != null
  const [localFilter, setLocalFilter] = useState('')
  const filterValue = hasSharedFilter ? sharedFilter : localFilter

  useEffect(() => {
    const metricsLogs = logs.filter(l => l.kind === 'metrics')
    if (metricsLogs.length === 0) return

    let dead = false
    Promise.all(metricsLogs.map(async (log) => {
      const res = await fetch(`${runFilesBase(run)}${log.path}`)
      if (!res.ok) return []
      const text = await res.text()
      const device = log.path.split('.')[1] || log.node // device.<name>.metrics.jsonl
      const points: MetricPoint[] = text.trim().split('\n')
        .map(line => { try { return JSON.parse(line) } catch { return null } })
        .filter((p): p is MetricPoint => p != null)

      // Group by key
      const byKey = new Map<string, { t: number; v: number }[]>()
      for (const p of points) {
        for (const [k, v] of Object.entries(p.m)) {
          if (typeof v !== 'number') continue
          let arr = byKey.get(k)
          if (!arr) { arr = []; byKey.set(k, arr) }
          arr.push({ t: p.t, v })
        }
      }
      return Array.from(byKey.entries()).map(([key, values]) => ({
        device, key, values
      }))
    })).then(results => {
      if (!dead) setSeries(results.flat())
    })

    return () => { dead = true }
  }, [run, logs])

  // Derive unique devices and metric keys, then pivot
  const devices = useMemo(() => {
    const set = new Set<string>()
    for (const s of series) set.add(s.device)
    return Array.from(set).sort()
  }, [series])

  const metricKeys = useMemo(() => {
    const set = new Set<string>()
    for (const s of series) set.add(s.key)
    return Array.from(set).sort()
  }, [series])

  // Build lookup: key -> device -> series
  const lookup = useMemo(() => {
    const map = new Map<string, Map<string, MetricSeries>>()
    for (const s of series) {
      let byDevice = map.get(s.key)
      if (!byDevice) { byDevice = new Map(); map.set(s.key, byDevice) }
      byDevice.set(s.device, s)
    }
    return map
  }, [series])

  // Filter metric keys
  const filteredKeys = useMemo(() => {
    if (!filterValue) return metricKeys
    const q = filterValue.toLowerCase()
    return metricKeys.filter(k => k.toLowerCase().includes(q))
  }, [metricKeys, filterValue])

  if (series.length === 0) {
    return <div className="empty">No metrics recorded for this run.</div>
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      {/* Filter input -- hidden when shared filter is provided */}
      {!hasSharedFilter && (
        <div style={{ padding: '8px 12px', borderBottom: '1px solid var(--border)', flexShrink: 0 }}>
          <input
            type="search"
            placeholder="Filter metrics by key..."
            value={localFilter}
            onChange={(e) => setLocalFilter(e.target.value)}
            style={{ width: '100%', maxWidth: 400 }}
          />
        </div>
      )}

      <div className="tbl-wrap">
        <table>
          <thead>
            <tr>
              <th>Metric</th>
              {devices.map(d => (
                <th key={d} colSpan={2}>{d}</th>
              ))}
            </tr>
            <tr>
              <th></th>
              {devices.map(d => (
                <Fragment key={d}>
                  <th style={{ fontSize: 11, fontWeight: 400, color: 'var(--text-muted)' }}>value</th>
                  <th style={{ fontSize: 11, fontWeight: 400, color: 'var(--text-muted)' }}>trend</th>
                </Fragment>
              ))}
            </tr>
          </thead>
          <tbody>
            {filteredKeys.map((key) => {
              const byDevice = lookup.get(key)
              return (
                <tr key={key}>
                  <td><code>{key}</code></td>
                  {devices.map(device => {
                    const s = byDevice?.get(device)
                    if (!s) {
                      return (
                        <Fragment key={device}>
                          <td style={{ color: 'var(--text-muted)' }}>&#8212;</td>
                          <td></td>
                        </Fragment>
                      )
                    }
                    const lastVal = s.values[s.values.length - 1]?.v
                    return (
                      <Fragment key={device}>
                        <td>{lastVal != null ? lastVal.toFixed(2) : '\u2014'}</td>
                        <td><Sparkline values={s.values.map(v => v.v)} /></td>
                      </Fragment>
                    )
                  })}
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}
