import { ResourceWorkspace } from '@/components/resource-workspace';
import { billingCredits } from '@/pages/billing/account-specs';

export const CreditsPage = () => <ResourceWorkspace spec={billingCredits} />;
