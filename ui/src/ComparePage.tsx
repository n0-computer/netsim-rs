import { useParams } from 'react-router-dom'
import CompareView from './components/CompareView'

export default function ComparePage() {
  const { left, right } = useParams<{ left: string; right: string }>()

  if (!left || !right) {
    return <div className="empty">Missing run names in URL. Use /compare/:left/:right</div>
  }

  return (
    <div className="app">
      <div className="topbar">
        <h1><a href="/" style={{ color: 'inherit', textDecoration: 'none' }}>patchbay</a></h1>
      </div>
      <CompareView leftRun={left} rightRun={right} />
    </div>
  )
}
