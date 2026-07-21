// 路由策略可视化拓扑编排 - 节点类型与画布配置

export type RouteNodeType =
  | 'inbound'         // 呼入源
  | 'prefix_match'    // 前缀匹配
  | 'time_filter'     // 时间段过滤 (时间路由)
  | 'caller_filter'   // 主叫过滤
  | 'lcr'             // 最低成本路由
  | 'gateway_trunk'   // 落地网关
  | 'ivr_branch'      // IVR 分支
  | 'queue_branch'    // 坐席队列分支
  | 'fork'            // 并行分支
  | 'reject';         // 拒绝

export interface RouteNode {
  id: string;
  type: RouteNodeType;
  title: string;
  description?: string;
  position: { x: number; y: number };
  config: Record<string, unknown>;
}

export interface RouteEdge {
  id: string;
  source: string;
  target: string;
  sourcePort?: string;
  label?: string;
}

export interface RouteTopology {
  id: string;
  name: string;
  description?: string;
  enabled: boolean;
  nodes: RouteNode[];
  edges: RouteEdge[];
  created_at?: string;
  updated_at?: string;
}

export interface RouteNodeCatalogEntry {
  type: RouteNodeType;
  title: string;
  description: string;
  icon: string;
  color: string;
  category: 'source' | 'filter' | 'action';
  defaultConfig: Record<string, unknown>;
  defaultPorts: { id: string; label: string; type: 'in' | 'out' }[];
}

export const ROUTE_NODE_CATALOG: RouteNodeCatalogEntry[] = [
  {
    type: 'inbound',
    title: '呼入源',
    description: 'DID 或中继入呼触发点',
    icon: 'PhoneCall',
    color: 'bg-emerald-500/15 text-emerald-600 border-emerald-500/30',
    category: 'source',
    defaultConfig: { source_type: 'did', did: '', trunk_id: '' },
    defaultPorts: [{ id: 'out', label: '进入', type: 'out' }],
  },
  {
    type: 'prefix_match',
    title: '前缀匹配',
    description: '按被叫号码前缀分流',
    icon: 'Hash',
    color: 'bg-blue-500/15 text-blue-600 border-blue-500/30',
    category: 'filter',
    defaultConfig: {
      prefixes: [
        { prefix: '86', label: '中国大陆' },
        { prefix: '1', label: '北美' },
      ],
    },
    defaultPorts: [
      { id: 'matched', label: '匹配', type: 'out' },
      { id: 'nomatch', label: '不匹配', type: 'out' },
    ],
  },
  {
    type: 'time_filter',
    title: '时间路由',
    description: '按时间段/星期过滤 (时间路由)',
    icon: 'Clock',
    color: 'bg-amber-500/15 text-amber-600 border-amber-500/30',
    category: 'filter',
    defaultConfig: {
      time_start: '09:00',
      time_end: '18:00',
      weekdays: [1, 2, 3, 4, 5],  // 周一到周五
      timezone: 'Asia/Shanghai',
    },
    defaultPorts: [
      { id: 'in_window', label: '时段内', type: 'out' },
      { id: 'out_window', label: '时段外', type: 'out' },
    ],
  },
  {
    type: 'caller_filter',
    title: '主叫过滤',
    description: '按主叫号码黑白名单过滤',
    icon: 'Shield',
    color: 'bg-rose-500/15 text-rose-600 border-rose-500/30',
    category: 'filter',
    defaultConfig: {
      mode: 'whitelist',
      patterns: ['13800138000', '139*'],
    },
    defaultPorts: [
      { id: 'pass', label: '通过', type: 'out' },
      { id: 'block', label: '拒绝', type: 'out' },
    ],
  },
  {
    type: 'lcr',
    title: 'LCR 选路',
    description: '最低成本路由策略选择',
    icon: 'Route',
    color: 'bg-purple-500/15 text-purple-600 border-purple-500/30',
    category: 'action',
    defaultConfig: {
      strategy: 'lowest_cost',
      max_hops: 3,
      fallback: 'reject',
    },
    defaultPorts: [
      { id: 'matched', label: '选中路由', type: 'out' },
      { id: 'fallback', label: '回退', type: 'out' },
    ],
  },
  {
    type: 'gateway_trunk',
    title: '落地网关',
    description: '通过指定 PSTN 中继落地',
    icon: 'PhoneForwarded',
    color: 'bg-pink-500/15 text-pink-600 border-pink-500/30',
    category: 'action',
    defaultConfig: {
      trunk_id: 'gw-telecom',
      priority: 100,
      weight: 100,
      cost: 0.1,
    },
    defaultPorts: [{ id: 'out', label: '完成', type: 'out' }],
  },
  {
    type: 'ivr_branch',
    title: 'IVR 分支',
    description: '转入 IVR 流程处理',
    icon: 'GitFork',
    color: 'bg-violet-500/15 text-violet-600 border-violet-500/30',
    category: 'action',
    defaultConfig: { ivr_id: 'ivr-main' },
    defaultPorts: [{ id: 'out', label: '进入 IVR', type: 'out' }],
  },
  {
    type: 'queue_branch',
    title: '坐席队列',
    description: '转入坐席队列接待',
    icon: 'Users',
    color: 'bg-orange-500/15 text-orange-600 border-orange-500/30',
    category: 'action',
    defaultConfig: { queue_id: 'queue-support', priority: 5 },
    defaultPorts: [{ id: 'out', label: '进入队列', type: 'out' }],
  },
  {
    type: 'fork',
    title: '并行分支',
    description: '同时尝试多条路径 (First-Win)',
    icon: 'Split',
    color: 'bg-cyan-500/15 text-cyan-600 border-cyan-500/30',
    category: 'action',
    defaultConfig: { strategy: 'first_win', timeout_secs: 30 },
    defaultPorts: [
      { id: 'branch-a', label: '分支 A', type: 'out' },
      { id: 'branch-b', label: '分支 B', type: 'out' },
      { id: 'branch-c', label: '分支 C', type: 'out' },
    ],
  },
  {
    type: 'reject',
    title: '拒绝',
    description: '拒绝当前呼叫',
    icon: 'Ban',
    color: 'bg-red-500/15 text-red-600 border-red-500/30',
    category: 'action',
    defaultConfig: { reason: 'busy', sip_code: 486 },
    defaultPorts: [],
  },
];

export const ROUTE_NODE_CATALOG_MAP: Record<RouteNodeType, RouteNodeCatalogEntry> =
  ROUTE_NODE_CATALOG.reduce((acc, entry) => {
    acc[entry.type] = entry;
    return acc;
  }, {} as Record<RouteNodeType, RouteNodeCatalogEntry>);

export const genRouteNodeId = (type: RouteNodeType): string =>
  `${type}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;

export const genRouteEdgeId = (): string =>
  `edge-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
