import { Routes, Route } from 'react-router-dom'
import Layout from './components/Layout'
import Dashboard from './pages/Dashboard'
import Cdr from './pages/Cdr'
import Users from './pages/Users'
import Gateways from './pages/Gateways'
import RoutesPage from './pages/Routes'
import Registrations from './pages/Registrations'
import ActiveCalls from './pages/ActiveCalls'
import Numbers from './pages/Numbers'
import Recordings from './pages/Recordings'
import Reports from './pages/Reports'
import Rates from './pages/Rates'
import Accounts from './pages/Accounts'

function App() {
  return (
    <Layout>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/dashboard" element={<Dashboard />} />
        <Route path="/active-calls" element={<ActiveCalls />} />
        <Route path="/numbers" element={<Numbers />} />
        <Route path="/cdr" element={<Cdr />} />
        <Route path="/users" element={<Users />} />
        <Route path="/gateways" element={<Gateways />} />
        <Route path="/routes" element={<RoutesPage />} />
        <Route path="/registrations" element={<Registrations />} />
        <Route path="/recordings" element={<Recordings />} />
        <Route path="/reports" element={<Reports />} />
        <Route path="/rates" element={<Rates />} />
        <Route path="/accounts" element={<Accounts />} />
      </Routes>
    </Layout>
  )
}

export default App
