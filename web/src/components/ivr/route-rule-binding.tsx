// 路由规则拓扑画布: 基于单条路由规则的字段双向绑定可视化编排
// 表格字段 (prefix/gateway_id/time_start/time_end/weekdays) ↔ 画布节点配置 双向同步
// 使用与 IVR 画布统一的蓝图风格设计

import { useMemo, useState } from 'react';
import { Button, Chip } from '@heroui/react';
import { CheckCircle2, GitBranch, Info } from 'lucide-react';
import { autoLayoutNodes, RouteCanvas, RouteNodePalette, RouteNodeInspector } from './route-canvas';
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
  void hasCaller;

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
        <Chip size="sm" variant="flat" color="primary">
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
  const initialTopology = useMemo(() => {
    const raw = topologyFromRule(rule);
    return {
      ...raw,
      nodes: autoLayoutNodes(raw.nodes, raw.edges),
    };
  }, [rule.id]);
  return (
    <RouteCanvasWithSync
      initialTopology={initialTopology}
      onApply={(topology) => onChange(ruleFromTopology(topology))}
      ruleId={rule.id}
    />
  );
}

// 内部: 拓扑画布 + 工具栏 + "应用"按钮
function RouteCanvasWithSync({
  initialTopology,
  onApply,
  ruleId,
}: {
  initialTopology: RouteTopology;
  onApply: (t: RouteTopology) => void;
  ruleId: string;
}) {
  const [topology, setTopology] = useState<RouteTopology>(initialTopology);
  const [applied, setApplied] = useState(false);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);

  const selectedNode = topology.nodes.find((n) => n.id === selectedNodeId) ?? null;

  const handleNodeChange = (updated: RouteNode) => {
    setTopology({
      ...topology,
      nodes: topology.nodes.map((n) => (n.id === updated.id ? updated : n)),
    });
  };

  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      {/* 顶部工具栏 - 蓝图风格 */}
      <div className="flex items-center justify-between gap-4 p-3 bg-content1 rounded-xl border border-default-200 shrink-0">
        <div className="flex items-center gap-3 flex-wrap min-w-0">
          <GitBranch className="w-5 h-5 text-primary shrink-0" />
          <Chip size="sm" variant="flat" color="primary" className="font-mono font-bold">
            {ruleId}
          </Chip>
          <Chip size="sm" variant="flat" color="primary">
            {topology.nodes.length} 节点 · {topology.edges.length} 连线
          </Chip>
          <div className="hidden sm:flex items-center gap-1.5 text-[10px] text-default-400">
            <Info className="w-3 h-3" />
            <span>拖拽节点 / 修改配置实时回写表格字段</span>
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          {applied && (
            <Chip size="sm" color="success" variant="flat" startContent={<CheckCircle2 className="w-3 h-3" />}>
              已应用
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

      {/* 三栏布局: 左侧 palette + 中间 canvas + 右侧 inspector */}
      <div className="flex gap-3 flex-1 min-h-0 h-full">
        <RouteNodePalette />
        <div className="flex-1 min-w-0 h-full">
          <RouteCanvas
            topology={topology}
            onChange={setTopology}
            selectedNodeId={selectedNodeId}
            onSelectNode={setSelectedNodeId}
          />
        </div>
        <RouteNodeInspector node={selectedNode} onChange={handleNodeChange} />
      </div>
    </div>
  );
}
