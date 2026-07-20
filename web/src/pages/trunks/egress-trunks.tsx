// 中继管理 - 落地中继列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { egressTrunks } from '@/pages/shared/resource-specs';

export const EgressTrunksPage = () => <ResourceWorkspace spec={egressTrunks} />;
