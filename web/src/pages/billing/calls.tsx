// 计费 - 通话记录
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { calls } from '@/pages/shared/resource-specs';

export const CallsPage = () => <ResourceWorkspace spec={calls} />;
