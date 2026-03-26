import React from 'react'
import ReactDOM from 'react-dom/client'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import App from './App'
import RunsIndex from './RunsIndex'
import ComparePage from './ComparePage'
import './index.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<RunsIndex />} />
        <Route path="/run/*" element={<App mode="run" />} />
        <Route path="/batch/*" element={<App mode="batch" />} />
        <Route path="/compare/:left/:right" element={<ComparePage />} />
        {/* Legacy redirect: /inv/:name → /batch/:name */}
        <Route path="/inv/*" element={<InvRedirect />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  </React.StrictMode>
)

/** Redirect legacy /inv/* paths to /batch/*. */
function InvRedirect() {
  const rest = window.location.pathname.slice('/inv/'.length)
  return <Navigate to={`/batch/${rest}`} replace />
}
