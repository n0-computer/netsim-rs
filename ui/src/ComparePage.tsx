import { useEffect, useState } from 'react'
import { useParams } from 'react-router-dom'
import { fetchRunJson } from './api'
import type { RunManifest } from './api'
import CompareView from './components/CompareView'

function refLabel(m: RunManifest | null): string | null {
  if (!m) return null
  if (m.branch && m.commit) return `${m.branch}@${m.commit.slice(0, 7)}`
  if (m.commit) return m.commit.slice(0, 7)
  return null
}

export default function ComparePage() {
  const { left, right } = useParams<{ left: string; right: string }>()
  const [leftManifest, setLeftManifest] = useState<RunManifest | null>(null)
  const [rightManifest, setRightManifest] = useState<RunManifest | null>(null)

  useEffect(() => {
    if (!left || !right) return
    fetchRunJson(left).then(setLeftManifest)
    fetchRunJson(right).then(setRightManifest)
  }, [left, right])

  if (!left || !right) {
    return <div className="empty">Missing run names in URL. Use /compare/:left/:right</div>
  }

  const project = leftManifest?.project ?? rightManifest?.project
  const leftRef = refLabel(leftManifest)
  const rightRef = refLabel(rightManifest)
  const subtitle = [
    project,
    leftRef && rightRef ? `${leftRef} vs ${rightRef}` : leftRef ?? rightRef,
  ].filter(Boolean).join(' · ')

  return (
    <div className="app">
      <div className="topbar">
        <h1><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>patchbay</a></h1>
        {subtitle && <span style={{ fontSize: 13, color: 'var(--text-muted)', marginLeft: '1rem' }}>{subtitle}</span>}
      </div>
      <CompareView leftRun={left} rightRun={right} />
    </div>
  )
}
