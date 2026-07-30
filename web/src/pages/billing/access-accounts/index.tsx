import { ResourceWorkspace } from '@/components/resource-workspace';
import { accessAccounts } from '@/pages/billing/account-specs';

export const AccessBillingAccountsPage = () => <ResourceWorkspace spec={accessAccounts} />;
