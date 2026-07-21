import { Navigate, Route, Routes } from 'react-router-dom';
import type { ReactNode } from 'react';
import { useAuth } from '@/auth/AuthContext';
import { canAccessPage } from '@/services/auth';
import ConsoleShell from '@/components/ConsoleShell';
import Login from '@/pages/Login';
import ErrorBoundary from '@/components/ErrorBoundary';
import { DashboardPage } from '@/pages/operations/dashboard';
import { ActiveCallsPage } from '@/pages/operations/active-calls';
import { CallDetailPage } from '@/pages/operations/call-detail';
import { ExtensionsPage } from '@/pages/numbers/extensions';
import { NumbersPage } from '@/pages/numbers/numbers';
import { DidDestinationsPage } from '@/pages/numbers/did-destinations';
import { CallerPoolsPage } from '@/pages/numbers/caller-pools';
import ExtensionDetailPage from '@/pages/numbers/extension-detail';
import CallerPoolDetailPage from '@/pages/numbers/caller-pool-detail';
import { AccessTrunksPage } from '@/pages/trunks/access-trunks';
import { EgressTrunksPage } from '@/pages/trunks/egress-trunks';
import { EgressGroupsPage } from '@/pages/trunks/egress-groups';
import TrunkDetailPage from '@/pages/trunks/trunk-detail';
import EgressGroupDetailPage from '@/pages/trunks/egress-group-detail';
import AgentsPage from '@/pages/call-center/agents';
import QueuesPage from '@/pages/call-center/queues';
import IvrPage from '@/pages/call-center/ivr';
import IvrEditorPage from '@/pages/call-center/ivr-editor';
import { AccountsPage } from '@/pages/billing/accounts';
import { RatesPage } from '@/pages/billing/rates';
import { TransactionsPage } from '@/pages/billing/transactions';
import { CallsPage } from '@/pages/billing/calls';
import { RoutesPage } from '@/pages/system/routes';
import { SecurityPage } from '@/pages/system/security';
import { InfrastructurePage } from '@/pages/system/infrastructure';
import { SettingsPage } from '@/pages/system/settings';

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
        <Route path="/did-destinations" element={<ProtectedPage path="/did-destinations"><DidDestinationsPage /></ProtectedPage>} />
        <Route path="/trunks/access" element={<ProtectedPage path="/trunks"><AccessTrunksPage /></ProtectedPage>} />
        <Route path="/trunks/egress" element={<ProtectedPage path="/trunks"><EgressTrunksPage /></ProtectedPage>} />
        <Route path="/trunks/access/:id" element={<ProtectedPage path="/trunks"><TrunkDetailPage /></ProtectedPage>} />
        <Route path="/trunks/egress/:id" element={<ProtectedPage path="/trunks"><TrunkDetailPage /></ProtectedPage>} />
        <Route path="/trunks/:id" element={<ProtectedPage path="/trunks"><TrunkDetailPage /></ProtectedPage>} />
        <Route path="/caller-pools" element={<ProtectedPage path="/caller-pools"><CallerPoolsPage /></ProtectedPage>} />
        <Route path="/caller-pools/:id" element={<ProtectedPage path="/caller-pools"><CallerPoolDetailPage /></ProtectedPage>} />
        <Route path="/egress-groups" element={<ProtectedPage path="/egress-groups"><EgressGroupsPage /></ProtectedPage>} />
        <Route path="/egress-groups/:id" element={<ProtectedPage path="/egress-groups"><EgressGroupDetailPage /></ProtectedPage>} />
        <Route path="/queues" element={<ProtectedPage path="/queues"><QueuesPage /></ProtectedPage>} />
        <Route path="/agents" element={<ProtectedPage path="/agents"><AgentsPage /></ProtectedPage>} />
        <Route path="/ivr" element={<ProtectedPage path="/ivr"><IvrPage /></ProtectedPage>} />
        <Route path="/ivr/:id/edit" element={<ProtectedPage path="/ivr"><IvrEditorPage /></ProtectedPage>} />
        <Route path="/ivr/:id/routes" element={<ProtectedPage path="/ivr"><IvrEditorPage /></ProtectedPage>} />

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
  return (
    <ErrorBoundary>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="*" element={<PrivateConsole />} />
      </Routes>
    </ErrorBoundary>
  );
}
