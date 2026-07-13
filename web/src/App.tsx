import { lazy, Suspense, type ReactNode } from 'react'
import { Routes, Route, Navigate } from 'react-router-dom'
import Layout from './components/Layout'
import Login from './pages/Login'
import { useAuth } from './auth/AuthContext'
import { canAccessPage, type UserRole } from './services/auth'
import AppErrorBoundary from './components/AppErrorBoundary'

const Dashboard = lazy(() => import('./pages/Dashboard'))
const Cdr = lazy(() => import('./pages/Cdr'))
const Users = lazy(() => import('./pages/Users'))
const Gateways = lazy(() => import('./pages/Gateways'))
const PeerGateways = lazy(() => import('./pages/PeerGateways'))
const RoutesPage = lazy(() => import('./pages/Routes'))
const Registrations = lazy(() => import('./pages/Registrations'))
const ActiveCalls = lazy(() => import('./pages/ActiveCalls'))
const Numbers = lazy(() => import('./pages/Numbers'))
const Reports = lazy(() => import('./pages/Reports'))
const Rates = lazy(() => import('./pages/Rates'))
const Accounts = lazy(() => import('./pages/Accounts'))
const AntiFraud = lazy(() => import('./pages/AntiFraud'))
const AuditLogs = lazy(() => import('./pages/AuditLogs'))
const SystemConfigs = lazy(() => import('./pages/SystemConfigs'))

function RequireAuth({ children }: { children: ReactNode }) {
  const { session } = useAuth();
  if (!session) return <Navigate to="/login" replace />;
  return <>{children}</>;
}

function Page({ path, roles, children }: { path: string; roles?: UserRole[]; children: ReactNode }) {
  const { session } = useAuth();
  const allowed = session && (roles ? roles.includes(session.role) : canAccessPage(session.role, path));
  return allowed ? <>{children}</> : (
    <div className="page-wrap access-denied">
      <div className="access-denied__icon">403</div>
      <h1>暂无访问权限</h1>
      <p>当前角色没有访问此页面的权限，请联系管理员开通。</p>
    </div>
  );
}

function App() {
  return (
    <Routes>
      <Route path="/login" element={<Login />} />
      <Route path="*" element={
        <RequireAuth>
          <Layout>
            <AppErrorBoundary>
              <Suspense fallback={<div className="loading-wrap" aria-live="polite">加载中...</div>}>
                <Routes>
                <Route path="/" element={<Navigate to="/dashboard" replace />} />
                <Route path="/dashboard" element={<Page path="/dashboard"><Dashboard /></Page>} />
                <Route path="/active-calls" element={<Page path="/active-calls"><ActiveCalls /></Page>} />
                <Route path="/users" element={<Page path="/users"><Users /></Page>} />
                <Route path="/gateways" element={<Page path="/gateways"><Gateways /></Page>} />
                <Route path="/peer-gateways" element={<Page path="/peer-gateways"><PeerGateways /></Page>} />
                <Route path="/routes" element={<Page path="/routes"><RoutesPage /></Page>} />
                <Route path="/registrations" element={<Page path="/registrations"><Registrations /></Page>} />
                <Route path="/numbers" element={<Page path="/numbers"><Numbers /></Page>} />
                <Route path="/cdr" element={<Page path="/cdr"><Cdr /></Page>} />
                <Route path="/reports" element={<Page path="/reports"><Reports /></Page>} />
                <Route path="/rates" element={<Page path="/rates"><Rates /></Page>} />
                <Route path="/accounts" element={<Page path="/accounts"><Accounts /></Page>} />
                <Route path="/anti-fraud" element={<Page path="/anti-fraud"><AntiFraud /></Page>} />
                <Route path="/audit-logs" element={<Page path="/audit-logs" roles={['admin']}><AuditLogs /></Page>} />
                <Route path="/system-configs" element={<Page path="/system-configs" roles={['admin']}><SystemConfigs /></Page>} />
                  <Route path="*" element={<Navigate to="/dashboard" replace />} />
                </Routes>
              </Suspense>
            </AppErrorBoundary>
          </Layout>
        </RequireAuth>
      } />
    </Routes>
  )
}

export default App
