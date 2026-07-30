import { ResourceWorkspace } from '@/components/resource-workspace';
import { egressAccounts } from '@/pages/billing/account-specs';

export const EgressBillingAccountsPage = () => <ResourceWorkspace spec={egressAccounts} />;
