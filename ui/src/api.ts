import type { LabEvent, LabState } from './devtools-types'
import type { CombinedResults, SimResults } from './types'

const API = '/api'

/** Manifest from run.json, included with pushed CI runs. */
export interface RunManifest {
  project: string
  branch?: string | null
  commit?: string | null
  pr?: number | null
  pr_url?: string | null
  created_at?: string | null
  title?: string | null
  /** CI test outcome (e.g. "success", "failure"). Not the lab lifecycle status. */
  test_outcome?: string | null
}

/** Metadata for a single Lab run directory. */
export interface RunInfo {
  name: string
  label: string | null
  status: string | null
  batch: string | null
  manifest?: RunManifest | null
}

/** A log file within a run directory. */
export interface LogEntry {
  node: string
  kind: string // 'tracing_jsonl' | 'jsonl' | 'json' | 'qlog' | 'ansi_text' | 'text'
  path: string
}

export async function fetchRuns(): Promise<RunInfo[]> {
  try {
    const res = await fetch(`${API}/runs`)
    if (!res.ok) return []
    return (await res.json()) as RunInfo[]
  } catch {
    return []
  }
}

export async function fetchState(run: string): Promise<LabState | null> {
  try {
    const res = await fetch(`${API}/runs/${encodeURIComponent(run)}/state`)
    if (!res.ok) return null
    return (await res.json()) as LabState
  } catch {
    return null
  }
}

export async function fetchEvents(run: string): Promise<LabEvent[]> {
  try {
    const res = await fetch(`${API}/runs/${encodeURIComponent(run)}/events.json`)
    if (!res.ok) return []
    return (await res.json()) as LabEvent[]
  } catch {
    return []
  }
}

export function subscribeEvents(
  run: string,
  afterOpid: number,
  onEvent: (event: LabEvent) => void,
): EventSource {
  const es = new EventSource(
    `${API}/runs/${encodeURIComponent(run)}/events?after=${afterOpid}`,
  )
  es.onmessage = (msg) => {
    try {
      onEvent(JSON.parse(msg.data))
    } catch {
      // ignore parse errors
    }
  }
  return es
}

export async function fetchLogs(run: string): Promise<LogEntry[]> {
  try {
    const res = await fetch(`${API}/runs/${encodeURIComponent(run)}/logs`)
    if (!res.ok) return []
    return (await res.json()) as LogEntry[]
  } catch {
    return []
  }
}

export async function fetchResults(run: string): Promise<SimResults | null> {
  try {
    const res = await fetch(
      `${API}/runs/${encodeURIComponent(run)}/files/results.json`,
    )
    if (!res.ok) return null
    return (await res.json()) as SimResults
  } catch {
    return null
  }
}

/** Base URL for fetching files within a run directory. */
export function runFilesBase(run: string): string {
  return `${API}/runs/${encodeURIComponent(run)}/files/`
}

export async function fetchCombinedResults(
  batch: string,
): Promise<CombinedResults | null> {
  try {
    const res = await fetch(
      `${API}/batches/${encodeURIComponent(batch)}/combined-results`,
    )
    if (!res.ok) return null
    return (await res.json()) as CombinedResults
  } catch {
    return null
  }
}
