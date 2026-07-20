// 计费 - 账务流水
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { transactions } from '@/pages/shared/resource-specs';

export const TransactionsPage = () => <ResourceWorkspace spec={transactions} />;
