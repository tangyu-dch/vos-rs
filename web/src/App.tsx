import { Navigate, Route, Routes, useLocation } from 'react-router-dom';
import type { ReactNode } from 'react';
import { useAuth } from '@/auth/AuthContext';
import { canAccessPage, firstMenuPath } from '@/services/auth';
import ConsoleShell from '@/components/ConsoleShell';
import Login from '@/pages/login';
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
import { AccessBillingAccountsPage } from '@/pages/billing/access-accounts';
import { EgressBillingAccountsPage } from '@/pages/billing/egress-accounts';
import { CreditsPage } from '@/pages/billing/credits';
import { TransactionsPage } from '@/pages/billing/transactions';
import { CallsPage } from '@/pages/billing/calls';
import { RoutesPage } from '@/pages/system/routes';
import { SecurityPage } from '@/pages/system/security';
import { InfrastructurePage } from '@/pages/system/infrastructure';
import { SettingsPage } from '@/pages/system/settings';
import { TenantsPage } from '@/pages/system/tenants';
import { LlmConfigPage } from '@/pages/settings/llm-config';

import { CopilotPage } from '@/pages/operations/copilot';
import { RwiConsolePage } from '@/pages/operations/rwi-console';
import { NotificationsPage } from '@/pages/operations/notifications';
import { AnnouncementsPage } from '@/pages/operations/announcements';
import { AccessAccountsPage } from '@/pages/system/access-accounts';
import { RolePermissionsPage } from '@/pages/system/role-permissions';
import { NotFoundPage } from '@/pages/not-found';
import { ProfilePage } from '@/pages/profile';

function PrivateConsole() {
  const { session } = useAuth();
  const location = useLocation();
  if (!session) return <Navigate to="/login" replace />;
  const homePath = firstMenuPath(session);
  return (
    <ConsoleShell>
      {/* key 随 pathname 变化触发容器重挂载，播放淡入过渡动画 */}
      <div key={location.pathname} className="page-transition flex-1 flex flex-col min-h-0">
        <Routes>
          <Route path="/" element={<Navigate to={homePath} replace />} />
          <Route path="/profile" element={<ProfilePage />} />
          <Route
            path="/overview"
            element={
              <ProtectedPage path="/overview">
                <DashboardPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/rwi"
            element={
              <ProtectedPage path="/rwi">
                <RwiConsolePage />
              </ProtectedPage>
            }
          />
          <Route
            path="/copilot"
            element={
              <ProtectedPage path="/copilot">
                <CopilotPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/notifications"
            element={
              <ProtectedPage path="/notifications">
                <NotificationsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/announcements"
            element={
              <ProtectedPage path="/announcements">
                <AnnouncementsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/calls/active"
            element={
              <ProtectedPage path="/calls/active">
                <ActiveCallsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/calls"
            element={
              <ProtectedPage path="/calls">
                <CallsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/calls/:id"
            element={
              <ProtectedPage path="/calls">
                <CallDetailPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/extensions"
            element={
              <ProtectedPage path="/extensions">
                <ExtensionsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/extensions/:id"
            element={
              <ProtectedPage path="/extensions">
                <ExtensionDetailPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/numbers"
            element={
              <ProtectedPage path="/numbers">
                <NumbersPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/did-destinations"
            element={
              <ProtectedPage path="/did-destinations">
                <DidDestinationsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/trunks/access"
            element={
              <ProtectedPage path="/trunks/access">
                <AccessTrunksPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/trunks/egress"
            element={
              <ProtectedPage path="/trunks/egress">
                <EgressTrunksPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/trunks/access/:id"
            element={
              <ProtectedPage path="/trunks/access">
                <TrunkDetailPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/trunks/egress/:id"
            element={
              <ProtectedPage path="/trunks/egress">
                <TrunkDetailPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/caller-pools"
            element={
              <ProtectedPage path="/caller-pools">
                <CallerPoolsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/caller-pools/:id"
            element={
              <ProtectedPage path="/caller-pools">
                <CallerPoolDetailPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/egress-groups"
            element={
              <ProtectedPage path="/egress-groups">
                <EgressGroupsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/egress-groups/:id"
            element={
              <ProtectedPage path="/egress-groups">
                <EgressGroupDetailPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/queues"
            element={
              <ProtectedPage path="/queues">
                <QueuesPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/agents"
            element={
              <ProtectedPage path="/agents">
                <AgentsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/ivr"
            element={
              <ProtectedPage path="/ivr">
                <IvrPage />
              </ProtectedPage>
            }
          />

          <Route
            path="/routing"
            element={
              <ProtectedPage path="/routing">
                <RoutesPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/billing/access-accounts"
            element={
              <ProtectedPage path="/billing/access-accounts">
                <AccessBillingAccountsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/billing/egress-accounts"
            element={
              <ProtectedPage path="/billing/egress-accounts">
                <EgressBillingAccountsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/billing/credits"
            element={
              <ProtectedPage path="/billing/credits">
                <CreditsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/billing/transactions"
            element={
              <ProtectedPage path="/billing/transactions">
                <TransactionsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/security"
            element={
              <ProtectedPage path="/security">
                <SecurityPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/infrastructure"
            element={
              <ProtectedPage path="/infrastructure">
                <InfrastructurePage />
              </ProtectedPage>
            }
          />
          <Route
            path="/tenants"
            element={
              <ProtectedPage path="/tenants">
                <TenantsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/settings"
            element={
              <ProtectedPage path="/settings">
                <SettingsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/settings/llm"
            element={
              <ProtectedPage path="/settings/llm">
                <LlmConfigPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/access-control/accounts"
            element={
              <ProtectedPage path="/access-control/accounts">
                <AccessAccountsPage />
              </ProtectedPage>
            }
          />
          <Route
            path="/access-control/roles"
            element={
              <ProtectedPage path="/access-control/roles">
                <RolePermissionsPage />
              </ProtectedPage>
            }
          />
          <Route path="*" element={<NotFoundPage />} />
        </Routes>
      </div>
    </ConsoleShell>
  );
}

function ProtectedPage({ path, children }: { path: string; children: ReactNode }) {
  const { session } = useAuth();
  return session && canAccessPage(session, path) ? (
    <>{children}</>
  ) : (
    <Navigate to={session ? firstMenuPath(session) : '/login'} replace />
  );
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
