import { Navigate, Route, Routes } from 'react-router-dom';
import type { ReactNode } from 'react';
import { useAuth } from './auth/AuthContext';
import { canAccessPage } from './services/auth';
import ConsoleShell from './components/ConsoleShell';
import Login from './pages/Login';
import {
  AccountsPage, ActiveCallsPage, CallDetailPage, CallsPage, DashboardPage,
  ExtensionDetailPage, ExtensionsPage, InfrastructurePage, NumbersPage,
  RatesPage, RoutesPage, SecurityPage, SettingsPage, TransactionsPage,
  CallerPoolsPage, EgressGroupsPage, TrunksPage,
} from './pages/console';
import TrunkDetailPage from './pages/trunk-detail';

function PrivateConsole() {
  const { session } = useAuth();
  if (!session) return <Navigate to="/login" replace />;
  return (
    <ConsoleShell>
      <Routes>
        <Route path="/" element={<Navigate to="/overview" replace />} />
        <Route path="/overview" element={<DashboardPage />} />
        <Route path="/calls/active" element={<ActiveCallsPage />} />
        <Route path="/calls" element={<CallsPage />} />
        <Route path="/calls/:id" element={<CallDetailPage />} />
        <Route path="/extensions" element={<ProtectedPage path="/extensions"><ExtensionsPage /></ProtectedPage>} />
        <Route path="/extensions/:id" element={<ProtectedPage path="/extensions"><ExtensionDetailPage /></ProtectedPage>} />
        <Route path="/numbers" element={<ProtectedPage path="/numbers"><NumbersPage /></ProtectedPage>} />
        <Route path="/trunks" element={<ProtectedPage path="/trunks"><TrunksPage /></ProtectedPage>} />
        <Route path="/trunks/:id" element={<ProtectedPage path="/trunks"><TrunkDetailPage /></ProtectedPage>} />
        <Route path="/caller-pools" element={<ProtectedPage path="/caller-pools"><CallerPoolsPage /></ProtectedPage>} />
        <Route path="/egress-groups" element={<ProtectedPage path="/egress-groups"><EgressGroupsPage /></ProtectedPage>} />
        <Route path="/routing" element={<ProtectedPage path="/routing"><RoutesPage /></ProtectedPage>} />
        <Route path="/billing/accounts" element={<ProtectedPage path="/billing/accounts"><AccountsPage /></ProtectedPage>} />
        <Route path="/billing/rates" element={<ProtectedPage path="/billing/rates"><RatesPage /></ProtectedPage>} />
        <Route path="/billing/transactions" element={<ProtectedPage path="/billing/transactions"><TransactionsPage /></ProtectedPage>} />
        <Route path="/security" element={<ProtectedPage path="/security"><SecurityPage /></ProtectedPage>} />
        <Route path="/infrastructure" element={<ProtectedPage path="/infrastructure"><InfrastructurePage /></ProtectedPage>} />
        <Route path="/settings" element={<ProtectedPage path="/settings"><SettingsPage /></ProtectedPage>} />
        <Route path="*" element={<Navigate to="/overview" replace />} />
      </Routes>
    </ConsoleShell>
  );
}

function ProtectedPage({ path, children }: { path: string; children: ReactNode }) {
  const { session } = useAuth();
  return session && canAccessPage(session.role, path) ? <>{children}</> : <Navigate to="/overview" replace />;
}

export default function App() {
  return <Routes><Route path="/login" element={<Login />} /><Route path="*" element={<PrivateConsole />} /></Routes>;
}
