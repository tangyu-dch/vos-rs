// 计费 - 费率列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { rates } from '@/pages/shared/resource-specs';

export const RatesPage = () => <ResourceWorkspace spec={rates} />;
