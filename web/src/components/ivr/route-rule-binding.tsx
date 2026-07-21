// 路由规则拓扑画布: 基于单条路由规则的字段双向绑定可视化编排
// 表格字段 (prefix/gateway_id/time_start/time_end/weekdays) ↔ 画布节点配置 双向同步

import { useMemo } from 'react';
import { Chip } from '@heroui/react';
import { RouteCanvas } from './route-canvas';
import {
  genRouteEdgeId,
  type RouteNode, type RouteTopology,
} from './route-types';

// 表格行数据类型
export interface RouteRuleFields {
  id: string;
  prefix?: string;
  priority?: number;
  gateway_id?: string;
  cost?: number;
  weight?: number;
  time_start?: string;
  time_end?: string;
  weekdays?: string;
  timezone?: string;
  caller_pattern?: string;
  failover_strategy?: string;
}

// 从表格字段生成默认拓扑 (规则首次打开画布时使用)
export function topologyFromRule(rule: RouteRuleFields): RouteTopology {
  const inboundId = `${rule.id}-inbound`;
  const prefixId = `${rule.id}-prefix`;
  const timeId = `${rule.id}-time`;
  const gwId = `${rule.id}-gateway`;
  const rejectId = `${rule.id}-reject`;
  const hasTime = rule.time_start || rule.time_end;
  const hasCaller = rule.caller_pattern;

  const nodes: RouteNode[] = [
    {
      id: inboundId,
      type: 'inbound',
      title: '呼入源',
      description: `规则 ${rule.id} 触发点`,
      position: { x: 60, y: 280 },
      config: { source_type: 'did', did: '', trunk_id: '' },
    },
    {
      id: prefixId,
      type: 'prefix_match',
      title: '前缀匹配',
      description: rule.prefix ? `匹配 ${rule.prefix}` : '全前缀匹配',
      position: { x: 360, y: 280 },
      config: {
        prefixes: rule.prefix
          ? [{ prefix: rule.prefix, label: `${rule.prefix}` }]
          : [{ prefix: '', label: '全前缀' }],
      },
    },
    {
      id: timeId,
      type: 'time_filter',
      title: '时间路由',
      description: hasTime ? `${rule.time_start ?? '--:--'} ~ ${rule.time_end ?? '--:--'}` : '未配置时间窗口',
      position: { x: 660, y: 280 },
      config: {
        time_start: rule.time_start ?? '09:00',
        time_end: rule.time_end ?? '18:00',
        weekdays: parseWeekdays(rule.weekdays),
        timezone: rule.timezone ?? 'Asia/Shanghai',
      },
    },
    {
      id: gwId,
      type: 'gateway_trunk',
      title: '落地网关',
      description: rule.gateway_id ? `→ ${rule.gateway_id}` : '未配置网关',
      position: { x: 960, y: 200 },
      config: {
        trunk_id: rule.gateway_id ?? '',
        priority: rule.priority ?? 100,
        weight: rule.weight ?? 100,
        cost: rule.cost ?? 0,
      },
    },
    {
      id: rejectId,
      type: 'reject',
      title: '拒绝/回退',
      description: rule.failover_strategy ? `策略: ${rule.failover_strategy}` : '默认回退',
      position: { x: 960, y: 380 },
      config: { reason: 'busy', sip_code: 486 },
    },
  ];

  const edges = [
    { id: genRouteEdgeId(), source: inboundId, target: prefixId, sourcePort: 'out', label: '进入' },
    { id: genRouteEdgeId(), source: prefixId, target: timeId, sourcePort: 'matched', label: '前缀匹配' },
    { id: genRouteEdgeId(), source: timeId, target: gwId, sourcePort: 'in_window', label: '时段内' },
    { id: genRouteEdgeId(), source: timeId, target: rejectId, sourcePort: 'out_window', label: '时段外' },
  ];

  // 如果有主叫过滤,在 prefix 和 time 之间插入 caller_filter (此处简化: 仅描述,不实际插入)
  void hasCaller;

  return {
    id: `topo-${rule.id}`,
    name: `规则 ${rule.id} 拓扑`,
    enabled: true,
    nodes,
    edges,
  };
}

// 从拓扑画布回写到表格字段
export function ruleFromTopology(topology: RouteTopology): Partial<RouteRuleFields> {
  const result: Partial<RouteRuleFields> = {};
  const prefixNode = topology.nodes.find((n) => n.type === 'prefix_match');
  const timeNode = topology.nodes.find((n) => n.type === 'time_filter');
  const gwNode = topology.nodes.find((n) => n.type === 'gateway_trunk');
  const rejectNode = topology.nodes.find((n) => n.type === 'reject');

  if (prefixNode) {
    const prefixes = prefixNode.config.prefixes as Array<{ prefix: string; label: string }> | undefined;
    const firstPrefix = prefixes?.[0]?.prefix;
    if (firstPrefix) result.prefix = firstPrefix;
  }
  if (timeNode) {
    result.time_start = String(timeNode.config.time_start ?? '');
    result.time_end = String(timeNode.config.time_end ?? '');
    const weekdays = timeNode.config.weekdays as number[] | undefined;
    if (weekdays && weekdays.length) result.weekdays = weekdays.join(',');
    result.timezone = String(timeNode.config.timezone ?? 'Asia/Shanghai');
  }
  if (gwNode) {
    result.gateway_id = String(gwNode.config.trunk_id ?? '');
    result.priority = Number(gwNode.config.priority ?? 100);
    result.weight = Number(gwNode.config.weight ?? 100);
    result.cost = Number(gwNode.config.cost ?? 0);
  }
  if (rejectNode) {
    const reason = String(rejectNode.config.reason ?? 'busy');
    result.failover_strategy = reason === 'busy' ? 'play_busy' : 'reject';
  }
  return result;
}

function parseWeekdays(s?: string): number[] {
  if (!s) return [1, 2, 3, 4, 5];
  return s.split(',').map((x) => Number(x.trim())).filter((n) => n >= 1 && n <= 7);
}

// 拓扑预览卡片 (在 Modal 中展示, 不允许拖拽, 仅查看)
export function RouteTopologyPreview({ topology }: { topology: RouteTopology }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2 flex-wrap">
        <Chip size="sm" variant="flat" color="secondary">
          {topology.nodes.length} 节点
        </Chip>
        <Chip size="sm" variant="flat" color="primary">
          {topology.edges.length} 连线
        </Chip>
        {topology.nodes.find((n) => n.type === 'time_filter') && (
          <Chip size="sm" variant="flat" color="warning">
            时间路由已启用
          </Chip>
        )}
      </div>
      <div className="h-[560px]">
        <RouteCanvas topology={topology} onChange={() => {}} />
      </div>
    </div>
  );
}

// 拓扑编辑画布 + 实时同步表格字段
interface RouteTopologyEditorProps {
  rule: RouteRuleFields;
  onChange: (updated: Partial<RouteRuleFields>) => void;
}

export function RouteTopologyEditor({ rule, onChange }: RouteTopologyEditorProps) {
  const initialTopology = useMemo(() => topologyFromRule(rule), [rule.id]);  // 仅依赖 id,避免每次字段变都重建
  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      <div className="flex items-center justify-between shrink-0">
        <div className="flex items-center gap-2">
          <Chip size="sm" variant="flat" color="secondary">规则: {rule.id}</Chip>
          <Chip size="sm" variant="flat" color="primary">
            拖拽节点 / 修改配置将实时回写到表格字段
          </Chip>
        </div>
        <span className="text-xs text-default-400">
          提示: 修改节点配置后,点击"应用拓扑到表格"按钮,配置将同步至表格字段
        </span>
      </div>
      <RouteCanvasWithSync
        initialTopology={initialTopology}
        onApply={(topology) => onChange(ruleFromTopology(topology))}
      />
    </div>
  );
}

// 内部: 拓扑画布 + "应用"按钮
import { useState } from 'react';
import { Button } from '@heroui/react';
import { CheckCircle2 } from 'lucide-react';

function RouteCanvasWithSync({
  initialTopology,
  onApply,
}: {
  initialTopology: RouteTopology;
  onApply: (t: RouteTopology) => void;
}) {
  const [topology, setTopology] = useState<RouteTopology>(initialTopology);
  const [applied, setApplied] = useState(false);
  return (
    <div className="flex flex-col gap-2 h-full min-h-0">
      <div className="flex-1 min-h-0">
        <RouteCanvas topology={topology} onChange={setTopology} />
      </div>
      <div className="flex items-center justify-end gap-2 shrink-0">
        {applied && (
          <Chip size="sm" color="success" variant="flat" startContent={<CheckCircle2 className="w-3 h-3" />}>
            已应用,请点击保存使配置生效
          </Chip>
        )}
        <Button
          size="sm"
          color="primary"
          className="font-bold text-white"
          startContent={<CheckCircle2 className="w-3.5 h-3.5" />}
          onPress={() => {
            onApply(topology);
            setApplied(true);
            setTimeout(() => setApplied(false), 2000);
          }}
        >
          应用拓扑到表格
        </Button>
      </div>
    </div>
  );
}
