import { useState, useCallback, useMemo } from 'react'
import type { LabEvent, LabState } from '../devtools-types'
import type { SimResults } from '../types'
import type { RunInfo, LogEntry } from '../api'
import { runFilesBase } from '../api'
import LogsTab from './LogsTab'
import PerfTab from './PerfTab'
import TimelineTab from './TimelineTab'
import TopologyGraph from './TopologyGraph'
import NodeDetail from './NodeDetail'
import MetricsTab from './MetricsTab'

export type RunTab = 'topology' | 'logs' | 'timeline' | 'perf' | 'metrics'

/** External controls passed from CompareView for shared filter state. */
export interface ExternalControls {
  logFilter?: string
  logLevels?: Set<string>
  metricsFilter?: string
}

interface RunViewProps {
  run: RunInfo
  state: LabState | null
  events: LabEvent[]
  logs: LogEntry[]
  results: SimResults | null
  activeTab: RunTab
  onTabChange: (tab: RunTab) => void
  externalControls?: ExternalControls
}

export default function RunView({ run, state, events, logs, results, activeTab, onTabChange, externalControls }: RunViewProps) {
  const [selectedNode, setSelectedNode] = useState<string | null>(null)
  const [selectedKind, setSelectedKind] = useState<'router' | 'device' | 'ix'>('router')
  const [logJump, setLogJump] = useState<{ node: string; path: string; timeLabel: string; nonce: number } | null>(null)

  const handleNodeSelect = useCallback((name: string, kind: 'router' | 'device' | 'ix') => {
    setSelectedNode(name)
    setSelectedKind(kind)
  }, [])

  const handleJumpToLog = useCallback((target: { node: string; path: string; timeLabel: string }) => {
    onTabChange('logs')
    setLogJump({ ...target, nonce: Date.now() })
  }, [onTabChange])

  const base = runFilesBase(run.name)
  const logsForTabs = useMemo(
    () => logs.map((l) => ({ node: l.node, kind: l.kind, path: l.path })),
    [logs],
  )

  const hasMetricsLogs = useMemo(() => logs.some(l => l.kind === 'metrics'), [logs])
  const availableTabs = useMemo<RunTab[]>(() => [
    'topology',
    'logs',
    'timeline',
    ...(results ? (['perf'] as RunTab[]) : []),
    ...(hasMetricsLogs ? (['metrics'] as RunTab[]) : []),
  ], [results, hasMetricsLogs])

  const tab = availableTabs.includes(activeTab) ? activeTab : availableTabs[0]

  return (
    <>
      <div className="tabs">
        {availableTabs.map((t) => (
          <button
            key={t}
            className={`tab-btn${tab === t ? ' active' : ''}`}
            onClick={() => onTabChange(t)}
          >
            {t}
          </button>
        ))}
      </div>

      <div className="tab-content" style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {tab === 'topology' && state && (
          <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
            <div style={{ flex: 1 }}>
              <TopologyGraph state={state} selectedNode={selectedNode} onNodeSelect={handleNodeSelect} />
            </div>
            {selectedNode && (
              <div
                style={{
                  width: 360,
                  borderLeft: '1px solid var(--border)',
                  overflow: 'auto',
                  padding: 12,
                  background: 'var(--surface)',
                }}
              >
                <NodeDetail state={state} selectedNode={selectedNode} selectedKind={selectedKind} />
              </div>
            )}
          </div>
        )}
        {tab === 'topology' && !state && (
          <div className="empty">Loading lab state...</div>
        )}

        {tab === 'logs' && (
          <LogsTab
            base={base}
            logs={logsForTabs}
            jumpTarget={logJump}
            sharedFilter={externalControls?.logFilter}
            sharedLevels={externalControls?.logLevels}
          />
        )}

        {tab === 'timeline' && (
          <TimelineTab base={base} logs={logsForTabs} labEvents={events} onJumpToLog={handleJumpToLog} />
        )}

        {tab === 'perf' && <PerfTab results={results} />}

        {tab === 'metrics' && (
          <MetricsTab
            run={run.name}
            logs={logs}
            sharedFilter={externalControls?.metricsFilter}
          />
        )}
      </div>
    </>
  )
}
