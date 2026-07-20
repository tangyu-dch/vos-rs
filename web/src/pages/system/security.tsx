// 系统管理 - 安全策略
// 从 console.tsx 拆分

import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import type { ResourceSpec } from '@/pages/shared/types';

export function SecurityPage() {
  const rules: ResourceSpec = {
    title: '安全策略', description: '管理防盗打规则并查看近期风险事件。', path: '/security/anti-fraud/policies',
    idKey: 'id', createLabel: '新建策略',
    fields: [
      { key: 'id', label: '策略 ID', required: true },
      { key: 'rule_type', label: '规则类型', kind: 'select', required: true, options: ['cps_limit', 'concurrent_limit', 'number_blocklist', 'ip_blocklist'] },
      { key: 'target_value', label: '目标值', kind: 'textarea', required: true, fullWidth: true },
      { key: 'limit_number', label: '阈值', kind: 'number' },
      { key: 'enabled', label: '启用', kind: 'switch', defaultValue: true },
    ],
  };
  return <ResourceWorkspace spec={rules} />;
}
