import React from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import RunPage from './RunPage'
import GroupPage from './GroupPage'
import RunsIndex from './RunsIndex'
import ComparePage from './ComparePage'
import './index.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<RunsIndex />} />
        <Route path="/run/*" element={<RunPage />} />
        <Route path="/group/*" element={<GroupPage />} />
        <Route path="/compare/:left/:right" element={<ComparePage />} />
        {/* Legacy redirect: /inv/:name -> /group/:name */}
        <Route path="/inv/*" element={<InvRedirect />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  </React.StrictMode>
)

/** Redirect legacy /inv/* paths to /group/*. */
function InvRedirect() {
  const rest = window.location.pathname.slice('/inv/'.length)
  return <Navigate to={`/group/${rest}`} replace />
}
