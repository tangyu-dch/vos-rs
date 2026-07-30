// 计费 - 通话记录
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/components/resource-workspace';
import { calls } from '@/pages/billing/record-specs';

export const CallsPage = () => <ResourceWorkspace spec={calls} />;
