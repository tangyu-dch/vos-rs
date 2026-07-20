// 中继管理 - 落地分组列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { egressGroups } from '@/pages/shared/resource-specs';

export const EgressGroupsPage = () => <ResourceWorkspace spec={egressGroups} />;
