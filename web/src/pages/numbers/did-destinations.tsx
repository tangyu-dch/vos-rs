// 号码管理 - 呼入目标
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { didDestinations } from '@/pages/shared/resource-specs';

export const DidDestinationsPage = () => <ResourceWorkspace spec={didDestinations} />;
