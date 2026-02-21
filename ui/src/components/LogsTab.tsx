import { useEffect, useMemo, useState } from 'react'
import type { SimLogEntry } from '../types'

const ANSI_RE = /\x1b\[[0-9;]*m/g
const TRACING_RE = /^(\d{4}-\d{2}-\d{2}T[\d:.]+Z)\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+([^\s:]+):\s*(.*)/
const PREVIEW_BYTES = 256 * 1024

type ParsedLine =
  | { type: 'tracing'; level: string; ts: string; target: string; msg: string }
  | { type: 'event'; kind: string; raw: string }
  | { type: 'raw'; raw: string }

type TransferEvent = {
  kind: string
  status?: string
  message?: string
}

type QlogEvent = {
  time?: number
  name?: string
}

type RenderMode = 'rendered' | 'raw'

type LogMeta = {
  size_bytes: number
  line_count: number
}

interface Props {
  base: string
  logs: SimLogEntry[]
}

function parseLine(raw: string): ParsedLine {
  const stripped = raw.replace(ANSI_RE, '')
  try {
    const v = JSON.parse(stripped) as Record<string, unknown>
    if (typeof v.kind === 'string') return { type: 'event', kind: v.kind, raw: stripped }
  } catch { }

  const m = stripped.match(TRACING_RE)
  if (m) return { type: 'tracing', ts: m[1], level: m[2], target: m[3], msg: m[4] }
  return { type: 'raw', raw: stripped }
}

function parseTransferEvents(text: string): TransferEvent[] {
  const events: TransferEvent[] = []
  for (const line of text.split('\n')) {
    const s = line.trim()
    if (!s) continue
    try {
      const v = JSON.parse(s) as Record<string, unknown>
      if (typeof v.kind === 'string') {
        events.push({
          kind: v.kind,
          status: typeof v.status === 'string' ? v.status : undefined,
          message: typeof v.message === 'string' ? v.message : undefined,
        })
      }
    } catch { }
  }
  return events
}

function parseQlogEvents(text: string): QlogEvent[] {
  const out: QlogEvent[] = []
  for (const line of text.split('\n')) {
    const s = line.trim().replace(/^\x1e/, '')
    if (!s) continue
    try {
      const v = JSON.parse(s) as Record<string, unknown>
      out.push({
        time: typeof v.time === 'number' ? v.time : undefined,
        name: typeof v.name === 'string' ? v.name : undefined,
      })
    } catch { }
  }
  return out
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`
}

async function fetchLogMeta(url: string): Promise<LogMeta> {
  const res = await fetch(`${url}?__meta=1`)
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`)
  }
  const body = await res.json() as { size_bytes?: number; line_count?: number }
  return {
    size_bytes: body.size_bytes ?? 0,
    line_count: body.line_count ?? 0,
  }
}

async function fetchRangePreview(url: string, sizeBytes: number): Promise<string> {
  const start = Math.max(0, sizeBytes - PREVIEW_BYTES)
  const end = Math.max(0, sizeBytes - 1)
  const range = sizeBytes > 0 ? `bytes=${start}-${end}` : `bytes=0-${PREVIEW_BYTES - 1}`
  const res = await fetch(url, {
    headers: {
      Range: range,
    },
  })
  if (!res.ok && res.status !== 206) {
    throw new Error(`HTTP ${res.status}`)
  }
  return await res.text()
}

export default function LogsTab({ base, logs }: Props) {
  const [active, setActive] = useState<SimLogEntry | null>(null)
  const [meta, setMeta] = useState<LogMeta | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [loadingMeta, setLoadingMeta] = useState(false)
  const [loadingContent, setLoadingContent] = useState(false)
  const [renderMode, setRenderMode] = useState<RenderMode>('rendered')

  useEffect(() => {
    setActive((prev) => {
      if (prev && logs.some((l) => l.path === prev.path)) return prev
      return logs[0] ?? null
    })
  }, [logs])

  useEffect(() => {
    if (!active) return
    let dead = false
    const url = `${base}${active.path}`
    setLoaded(false)
    setText('')
    setError(null)
    setMeta(null)
    setLoadingMeta(true)
    setLoadingContent(false)
    setRenderMode(active.kind === 'text' ? 'raw' : 'rendered')
    fetchLogMeta(url)
      .then((m) => {
        if (dead) return
        setMeta(m)
      })
      .catch((e) => {
        if (dead) return
        setError(String(e))
      })
      .finally(() => {
        if (!dead) setLoadingMeta(false)
      })
    return () => {
      dead = true
    }
  }, [active, base])

  const loadPreview = async () => {
    if (!active) return
    const url = `${base}${active.path}`
    setLoadingContent(true)
    setError(null)
    try {
      const content = await fetchRangePreview(url, meta?.size_bytes ?? 0)
      setText(content)
      setLoaded(true)
    } catch (e) {
      setError(String(e))
    } finally {
      setLoadingContent(false)
    }
  }

  const byNode = useMemo(() => {
    const m = new Map<string, SimLogEntry[]>()
    for (const log of logs) {
      if (!m.has(log.node)) m.set(log.node, [])
      m.get(log.node)!.push(log)
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]))
  }, [logs])

  const parsed = useMemo(() => text.split('\n').filter(Boolean).map(parseLine), [text])
  const transferEvents = useMemo(() => parseTransferEvents(text), [text])
  const qlogEvents = useMemo(() => parseQlogEvents(text), [text])
  const supportsRendered = active?.kind === 'transfer' || active?.kind === 'qlog'

  return (
    <div className="logs-layout">
      <div className="logs-sidebar">
        {byNode.map(([node, files]) => (
          <div key={node} className="node-group">
            <div className="node-label">{node}</div>
            {files.map((f) => (
              <div
                key={f.path}
                className={`file-item${active?.path === f.path ? ' active' : ''}`}
                onClick={() => setActive(f)}
                title={f.path}
              >
                {f.path.split('/').pop()}
                <span style={{ marginLeft: 6, color: 'var(--text-muted)' }}>[{f.kind}]</span>
              </div>
            ))}
          </div>
        ))}
      </div>

      <div className="logs-main">
        {error && <div className="error-msg">{error}</div>}
        {!active && <div className="empty">no logs</div>}
        {active && (
          <>
            <div className="logs-toolbar">
              <span>{active.path}</span>
              {meta && (
                <span style={{ color: 'var(--text-muted)' }}>
                  {formatBytes(meta.size_bytes)} · {meta.line_count} lines
                </span>
              )}
              {supportsRendered && loaded && (
                <>
                  <button
                    className={`btn${renderMode === 'rendered' ? ' active' : ''}`}
                    onClick={() => setRenderMode('rendered')}
                  >
                    preview
                  </button>
                  <button
                    className={`btn${renderMode === 'raw' ? ' active' : ''}`}
                    onClick={() => setRenderMode('raw')}
                  >
                    raw
                  </button>
                </>
              )}
              {!loaded && (
                <button className="btn" onClick={loadPreview} disabled={loadingMeta || loadingContent}>
                  {loadingContent ? 'loading…' : 'load preview'}
                </button>
              )}
            </div>

            {!loaded && (
              <div className="empty">
                {loadingMeta ? 'reading metadata…' : 'load preview to view this log'}
              </div>
            )}

            {loaded && renderMode === 'rendered' && active.kind === 'transfer' && (
              <div className="tbl-wrap">
                <table>
                  <thead>
                    <tr>
                      <th>event</th>
                      <th>status</th>
                      <th>message</th>
                    </tr>
                  </thead>
                  <tbody>
                    {transferEvents.map((ev, i) => (
                      <tr key={i}>
                        <td>{ev.kind}</td>
                        <td>{ev.status ?? '—'}</td>
                        <td>{ev.message ?? '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            {loaded && renderMode === 'rendered' && active.kind === 'qlog' && (
              <div className="tbl-wrap">
                <table>
                  <thead>
                    <tr>
                      <th>time</th>
                      <th>name</th>
                    </tr>
                  </thead>
                  <tbody>
                    {qlogEvents.map((ev, i) => (
                      <tr key={i}>
                        <td>{ev.time ?? '—'}</td>
                        <td>{ev.name ?? '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            {loaded && (!supportsRendered || renderMode === 'raw') && (
              <div className="logs-content">
                {parsed.map((line, i) => {
                  if (line.type === 'tracing') {
                    return (
                      <div key={i} className="log-entry">
                        <span className="log-ts">{line.ts.split('T')[1]?.replace('Z', '')}</span>
                        <span className={`level-${line.level}`} style={{ marginRight: 8 }}>{line.level}</span>
                        <span className="log-target">{line.target}:</span>
                        <span className="log-msg">{line.msg}</span>
                      </div>
                    )
                  }
                  if (line.type === 'event') {
                    return <div key={i} className="log-entry log-iroh-events">{line.kind} {line.raw}</div>
                  }
                  return <div key={i} className="log-entry log-raw">{line.raw}</div>
                })}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  )
}
