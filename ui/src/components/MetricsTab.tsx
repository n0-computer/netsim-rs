import { useEffect, useState } from 'react'
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

export default function MetricsTab({ run, logs }: { run: string; logs: LogEntry[] }) {
  const [series, setSeries] = useState<MetricSeries[]>([])

  useEffect(() => {
    const metricsLogs = logs.filter(l => l.kind === 'metrics')
    if (metricsLogs.length === 0) return

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
    })).then(results => setSeries(results.flat()))
  }, [run, logs])

  if (series.length === 0) {
    return <div className="empty">No metrics recorded for this run.</div>
  }

  return (
    <div className="tbl-wrap">
      <table>
        <thead>
          <tr>
            <th>Key</th>
            <th>Device</th>
            <th>Last Value</th>
            <th>Trend</th>
          </tr>
        </thead>
        <tbody>
          {series.map((s, i) => (
            <tr key={`${s.device}:${s.key}`}>
              <td><code>{s.key}</code></td>
              <td>{s.device}</td>
              <td>{s.values[s.values.length - 1]?.v.toFixed(2)}</td>
              <td><Sparkline values={s.values.map(v => v.v)} /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
