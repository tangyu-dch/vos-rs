import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { accessAccounts } from '@/pages/billing/account-specs';

export const AccessBillingAccountsPage = () => <ResourceWorkspace spec={accessAccounts} />;
