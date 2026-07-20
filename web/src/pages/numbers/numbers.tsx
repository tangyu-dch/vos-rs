// 号码管理 - 号码库存
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { numbers } from '@/pages/shared/resource-specs';

export const NumbersPage = () => <ResourceWorkspace spec={numbers} />;
