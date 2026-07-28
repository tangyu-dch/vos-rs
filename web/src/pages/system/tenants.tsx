// 租户管理 - 多租户隔离策略、并发/CPS 上限、计费账户关联

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { tenants } from '@/pages/shared/resource-specs';

export function TenantsPage() {
  return <ResourceWorkspace spec={tenants} />;
}
