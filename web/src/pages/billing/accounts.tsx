// 计费 - 账户列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { accounts } from '@/pages/shared/resource-specs';

export const AccountsPage = () => <ResourceWorkspace spec={accounts} />;
