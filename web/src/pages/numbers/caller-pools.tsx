// 号码管理 - 号码池组列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { callerPools } from '@/pages/shared/resource-specs';

export const CallerPoolsPage = () => <ResourceWorkspace spec={callerPools} />;
