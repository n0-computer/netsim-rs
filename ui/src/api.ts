import type { LabEvent, LabState } from './devtools-types'
import type { CombinedResults, SimResults } from './types'

const API = '/api'

/** Test result entry within a run manifest. */
export interface TestResult {
  name: string
  status: string  // "pass" | "fail" | "ignored"
  duration?: number | null
}

/** Manifest from run.json, included with pushed CI runs. */
export interface RunManifest {
  kind?: string | null       // "test" | "sim"
  project?: string | null
  branch?: string | null
  commit?: string | null
  dirty?: boolean
  pr?: number | null
  pr_url?: string | null
  created_at?: string | null
  started_at?: string | null
  ended_at?: string | null
  runtime?: number | null
  title?: string | null
  /** CI test outcome (e.g. "success", "failure"). Not the lab lifecycle status. */
  test_outcome?: string | null
  outcome?: string | null
  pass?: number | null
  fail?: number | null
  total?: number | null
  tests?: TestResult[]
}

/** Metadata for a single Lab run directory. */
export interface RunInfo {
  name: string
  label: string | null
  status: string | null
  /** Group name (first path component for nested runs). */
  group: string | null
  manifest?: RunManifest | null
}

/** A log file within a run directory. */
export interface LogEntry {
  node: string
  kind: string // 'tracing_jsonl' | 'jsonl' | 'json' | 'qlog' | 'ansi_text' | 'text'
  path: string
}

export async function fetchRuns(params?: {
  project?: string
  kind?: string
  limit?: number
  offset?: number
}): Promise<RunInfo[]> {
  try {
    const query = new URLSearchParams()
    if (params?.project) query.set('project', params.project)
    if (params?.kind) query.set('kind', params.kind)
    if (params?.limit != null) query.set('limit', String(params.limit))
    if (params?.offset != null) query.set('offset', String(params.offset))
    const qs = query.toString()
    const res = await fetch(`${API}/runs${qs ? '?' + qs : ''}`)
    if (!res.ok) return []
    const raw = (await res.json()) as any[]
    // Normalize: accept both "group" and legacy "batch" from server
    return raw.map((r) => ({
      ...r,
      group: r.group ?? r.batch ?? null,
    })) as RunInfo[]
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

/** Fetch run.json manifest for a given run. */
export async function fetchRunJson(run: string): Promise<RunManifest | null> {
  try {
    const res = await fetch(`${runFilesBase(run)}run.json`)
    if (!res.ok) return null
    return (await res.json()) as RunManifest
  } catch {
    return null
  }
}

export async function fetchCombinedResults(
  group: string,
): Promise<CombinedResults | null> {
  try {
    const res = await fetch(
      `${API}/groups/${encodeURIComponent(group)}/combined-results`,
    )
    if (!res.ok) return null
    return (await res.json()) as CombinedResults
  } catch {
    return null
  }
}
