// 中继管理 - 接入中继列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { accessTrunks } from '@/pages/shared/resource-specs';

export const AccessTrunksPage = () => <ResourceWorkspace spec={accessTrunks} />;
