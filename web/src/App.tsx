import { lazy, Suspense } from 'react'
import { Routes, Route, Navigate } from 'react-router-dom'
import Layout from './components/Layout'

const Dashboard = lazy(() => import('./pages/Dashboard'))
const Cdr = lazy(() => import('./pages/Cdr'))
const Users = lazy(() => import('./pages/Users'))
const Gateways = lazy(() => import('./pages/Gateways'))
const PeerGateways = lazy(() => import('./pages/PeerGateways'))
const RoutesPage = lazy(() => import('./pages/Routes'))
const Registrations = lazy(() => import('./pages/Registrations'))
const ActiveCalls = lazy(() => import('./pages/ActiveCalls'))
const Numbers = lazy(() => import('./pages/Numbers'))
const Recordings = lazy(() => import('./pages/Recordings'))
const Reports = lazy(() => import('./pages/Reports'))
const Rates = lazy(() => import('./pages/Rates'))
const Accounts = lazy(() => import('./pages/Accounts'))
const AntiFraud = lazy(() => import('./pages/AntiFraud'))

function App() {
  return (
    <Layout>
      <Suspense fallback={<div className="loading-wrap" aria-live="polite">加载中...</div>}>
        <Routes>
          <Route path="/" element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<Dashboard />} />
          <Route path="/active-calls" element={<ActiveCalls />} />
          <Route path="/users" element={<Users />} />
          <Route path="/gateways" element={<Gateways />} />
          <Route path="/peer-gateways" element={<PeerGateways />} />
          <Route path="/routes" element={<RoutesPage />} />
          <Route path="/registrations" element={<Registrations />} />
          <Route path="/numbers" element={<Numbers />} />
          <Route path="/cdr" element={<Cdr />} />
          <Route path="/reports" element={<Reports />} />
          <Route path="/rates" element={<Rates />} />
          <Route path="/accounts" element={<Accounts />} />
          <Route path="/recordings" element={<Recordings />} />
          <Route path="/anti-fraud" element={<AntiFraud />} />
        </Routes>
      </Suspense>
    </Layout>
  )
}

export default App
