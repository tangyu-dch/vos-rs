// 号码管理 - 分机列表
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { extensions } from '@/pages/shared/resource-specs';

export const ExtensionsPage = () => <ResourceWorkspace spec={extensions} />;
