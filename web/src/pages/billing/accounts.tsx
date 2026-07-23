import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { accounts } from '@/pages/shared/resource-specs';

export function AccountsPage() {
  return <ResourceWorkspace spec={accounts} />;
}
