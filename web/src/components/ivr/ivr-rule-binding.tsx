// IVR 拓扑编辑器: 表格行点击「拓扑编排」后弹出的画布编辑器
// 与 route-rule-binding.tsx 对应, 但 IVR 拓扑复杂度高, 无法用表格字段表达
// 因此采用「表格字段 → 默认拓扑生成」+「画布编辑 → 直接保存后端」模式
// 表格仅保存 IVR 基础信息 (id/name/did/welcome_prompt/timeout), 节点拓扑由画布独立编辑保存
//
// 后端 JSON 字段使用 snake_case (source_port), 前端 IvrEdge 使用 camelCase (sourcePort)
// 在 load/save 时做转换

import { useCallback, useEffect, useMemo, useState } from 'react';
import { Button, Chip, Input } from '@heroui/react';
import { Save, Play, GitFork, AlertCircle, AlertTriangle, CheckCircle2, ShieldAlert } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState, LoadingState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import { autoLayoutNodes, IvrCanvas, NodeInspector, NodePalette } from './ivr-canvas';
import {
  NODE_CATALOG_MAP, genEdgeId, genNodeId,
  type IvrEdge, type IvrFlow, type IvrNode,
} from './types';

// 表格行字段 (从 IVR 列表项提取, 不含 nodes/edges)
export interface IvrFlowFields {
  id: string;
  name: string;
  description?: string;
  did?: string;
  welcome_prompt?: string;
  timeout_secs?: number;
  enabled?: boolean;
}

// 后端 IvrEdge DTO (snake_case)
interface IvrEdgeDto {
  id: string;
  source: string;
  target: string;
  source_port?: string | null;
  label?: string | null;
}

// 后端 IvrNode DTO (type 字段为关键字, 需要重命名)
interface IvrNodeDto {
  id: string;
  type: string;
  title: string;
  description?: string;
  position: { x: number; y: number };
  config: Record<string, unknown>;
}

export interface IvrValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
  orphanNodeIds: string[];
  cyclicNodeIds: string[];
}

/** IVR 流程图节点合法性与防呆校验 (孤立节点 / 死循环 / 缺少入口) */
export function validateIvrFlow(flow: IvrFlow): IvrValidationResult {
  const errors: string[] = [];
  const warnings: string[] = [];
  const orphanNodeIds: string[] = [];
  const cyclicNodeIds: string[] = [];

  const { nodes, edges } = flow;

  if (nodes.length === 0) {
    return {
      valid: false,
      errors: ['IVR 流程节点不可为空，请至少添加一个【呼入入口】节点'],
      warnings: [],
      orphanNodeIds: [],
      cyclicNodeIds: [],
    };
  }

  // 1. 检查是否存在入口节点
  const startNodes = nodes.filter((n) => n.type === 'start');
  if (startNodes.length === 0) {
    errors.push('流程缺少【呼入入口】(start) 节点，无法响应呼入路由');
  }

  // 构建邻接表
  const adj = new Map<string, string[]>();
  nodes.forEach((n) => adj.set(n.id, []));

  edges.forEach((e) => {
    if (adj.has(e.source) && adj.has(e.target)) {
      adj.get(e.source)!.push(e.target);
    }
  });

  // 2. 从 start 节点做 BFS 可达性分析 (查找孤立/不可达节点)
  const visitedFromStart = new Set<string>();
  const queue: string[] = startNodes.map((n) => n.id);
  queue.forEach((id) => visitedFromStart.add(id));

  while (queue.length > 0) {
    const curr = queue.shift()!;
    const neighbors = adj.get(curr) || [];
    for (const next of neighbors) {
      if (!visitedFromStart.has(next)) {
        visitedFromStart.add(next);
        queue.push(next);
      }
    }
  }

  nodes.forEach((n) => {
    if (!visitedFromStart.has(n.id)) {
      orphanNodeIds.push(n.id);
      errors.push(`检测到孤立游离节点: 【${n.title}】(${n.id})，从入口无法到达`);
    }
  });

  // 3. 悬空分支 / 未连接输出校验
  const terminalTypes = ['hangup', 'transfer_queue', 'transfer_ext', 'transfer_pstn', 'voicemail'];
  nodes.forEach((n) => {
    if (visitedFromStart.has(n.id) && !terminalTypes.includes(n.type)) {
      const outEdges = edges.filter((e) => e.source === n.id);
      if (outEdges.length === 0) {
        warnings.push(`节点【${n.title}】未配置输出连线或终结处理`);
      }
    }
  });

  // 4. 死循环 / 环路结构校验 (DFS Cycle Detection)
  const state = new Map<string, 'unvisited' | 'visiting' | 'visited'>();
  nodes.forEach((n) => state.set(n.id, 'unvisited'));

  const cycles: string[][] = [];
  const currentPath: string[] = [];

  function dfs(u: string) {
    state.set(u, 'visiting');
    currentPath.push(u);

    const neighbors = adj.get(u) || [];
    for (const v of neighbors) {
      if (state.get(v) === 'visiting') {
        const cycleStartIndex = currentPath.indexOf(v);
        if (cycleStartIndex !== -1) {
          cycles.push(currentPath.slice(cycleStartIndex));
        }
      } else if (state.get(v) === 'unvisited') {
        dfs(v);
      }
    }

    currentPath.pop();
    state.set(u, 'visited');
  }

  startNodes.forEach((n) => {
    if (state.get(n.id) === 'unvisited') {
      dfs(n.id);
    }
  });

  cycles.forEach((cycle) => {
    const hasLoopNode = cycle.some((id) => nodes.find((n) => n.id === id)?.type === 'loop');
    cycle.forEach((id) => {
      if (!cyclicNodeIds.includes(id)) cyclicNodeIds.push(id);
    });

    const cycleNames = cycle.map((id) => nodes.find((n) => n.id === id)?.title || id).join(' -> ');
    if (!hasLoopNode) {
      errors.push(`检测到死循环/非法环路: 【${cycleNames} -> ${nodes.find((n) => n.id === cycle[0])?.title}】，缺少显式【循环跳转】节点或退出条件`);
    } else {
      warnings.push(`流程包含受控循环: ${cycleNames}`);
    }
  });

  return {
    valid: errors.length === 0,
    errors,
    warnings,
    orphanNodeIds,
    cyclicNodeIds,
  };
}

// DTO → 前端 IvrEdge (snake_case → camelCase)
function edgeFromDto(dto: IvrEdgeDto): IvrEdge {
  return {
    id: dto.id,
    source: dto.source,
    target: dto.target,
    sourcePort: dto.source_port ?? undefined,
    label: dto.label ?? undefined,
  };
}

// 前端 IvrEdge → DTO (camelCase → snake_case)
function edgeToDto(edge: IvrEdge): IvrEdgeDto {
  return {
    id: edge.id,
    source: edge.source,
    target: edge.target,
    source_port: edge.sourcePort ?? null,
    label: edge.label ?? null,
  };
}

// DTO → 前端 IvrNode
function nodeFromDto(dto: IvrNodeDto): IvrNode {
  return {
    id: dto.id,
    type: dto.type as IvrNode['type'],
    title: dto.title,
    description: dto.description,
    position: dto.position,
    config: dto.config,
  };
}

// 从表格字段生成默认 IVR 流程 (首次打开画布 / 后端无 nodes 时使用)
export function flowFromFields(fields: IvrFlowFields): IvrFlow {
  const startNode: IvrNode = {
    id: genNodeId('start'),
    type: 'start',
    title: '呼入入口',
    description: `匹配 DID ${fields.did ?? '未指定'}`,
    position: { x: 80, y: 240 },
    config: { did: fields.did ?? '', welcome_prompt: fields.welcome_prompt ?? 'welcome.wav' },
  };
  const menuNode: IvrNode = {
    id: genNodeId('menu'),
    type: 'menu',
    title: '多级菜单',
    description: '主菜单分支',
    position: { x: 380, y: 240 },
    config: { ...NODE_CATALOG_MAP.menu.defaultConfig },
  };
  const hangupNode: IvrNode = {
    id: genNodeId('hangup'),
    type: 'hangup',
    title: '挂断',
    description: '结束通话',
    position: { x: 700, y: 240 },
    config: { reason: 'normal', playbye: true },
  };
  return {
    id: fields.id,
    name: fields.name,
    description: fields.description,
    did: fields.did,
    welcome_prompt: fields.welcome_prompt,
    timeout_secs: fields.timeout_secs ?? 30,
    enabled: fields.enabled ?? true,
    nodes: [startNode, menuNode, hangupNode],
    edges: [
      { id: genEdgeId(), source: startNode.id, target: menuNode.id, sourcePort: 'out', label: '进入' },
      { id: genEdgeId(), source: menuNode.id, target: hangupNode.id, sourcePort: 'key-0', label: '按 0' },
    ],
  };
}

// IVR 拓扑编辑器 (Modal 内使用)
interface IvrTopologyEditorProps {
  flow: IvrFlowFields;
  onSaved?: () => void;
}

export function IvrTopologyEditor({ flow, onSaved }: IvrTopologyEditorProps) {
  const [topology, setTopology] = useState<IvrFlow | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [ivrName, setIvrName] = useState(flow.name);

  // 加载 IVR 完整流程
  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const res: Record<string, unknown> = await api.get(`/ivr/menus/${flow.id}`);
      const menu = (res.data ?? res) as Record<string, unknown>;
      const menuNodeDtos = Array.isArray(menu.nodes) ? menu.nodes as IvrNodeDto[] : [];
      const menuEdgeDtos = Array.isArray(menu.edges) ? menu.edges as IvrEdgeDto[] : [];

      const nodes: IvrNode[] = menuNodeDtos.length > 0
        ? menuNodeDtos.map(nodeFromDto)
        : flowFromFields(flow).nodes;
      const edges: IvrEdge[] = menuNodeDtos.length > 0
        ? menuEdgeDtos.map(edgeFromDto)
        : flowFromFields(flow).edges;

      const layoutedNodes = autoLayoutNodes(nodes, edges);

      setTopology({
        id: String(menu.id ?? flow.id),
        name: String(menu.name ?? flow.name),
        description: menu.description as string | undefined,
        did: menu.did as string | undefined,
        welcome_prompt: menu.welcome_prompt as string | undefined,
        timeout_secs: Number(menu.timeout_secs ?? 30),
        enabled: Boolean(menu.enabled ?? true),
        nodes: layoutedNodes,
        edges,
      });
      setIvrName(String(menu.name ?? flow.name));
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载 IVR 流程失败');
    } finally {
      setLoading(false);
    }
  }, [flow.id, flow.name]);

  useEffect(() => { void load(); }, [load]);

  // 实时节点树图验证
  const validation = useMemo(() => (topology ? validateIvrFlow(topology) : null), [topology]);

  const handleSave = async () => {
    if (!topology) return;

    if (validation && !validation.valid) {
      message.error(validation.errors[0] || 'IVR 流程树包含非法错误，无法保存');
      return;
    }

    setSaving(true);
    try {
      const payload = {
        id: topology.id,
        name: ivrName,
        description: topology.description ?? '',
        did: topology.did ?? '',
        welcome_prompt: topology.welcome_prompt ?? 'welcome.wav',
        timeout_secs: topology.timeout_secs,
        enabled: topology.enabled,
        nodes: topology.nodes.map((n) => ({
          id: n.id,
          type: n.type,
          title: n.title,
          description: n.description ?? '',
          position: n.position,
          config: n.config,
        })),
        edges: topology.edges.map(edgeToDto),
        mappings: [],
      };
      await api.put(`/ivr/menus/${topology.id}`, payload);
      message.success('IVR 流程已保存');
      onSaved?.();
    } catch (err) {
      message.error(err instanceof Error ? err.message : '保存失败');
    } finally {
      setSaving(false);
    }
  };

  const selectedNode = topology?.nodes.find((n) => n.id === selectedNodeId) ?? null;

  const handleNodeChange = (updated: IvrNode) => {
    if (!topology) return;
    setTopology({
      ...topology,
      nodes: topology.nodes.map((n) => (n.id === updated.id ? updated : n)),
    });
  };

  if (loading) {
    return (
      <div className="h-full min-h-[400px] flex items-center justify-center">
        <LoadingState />
      </div>
    );
  }

  if (error) {
    return (
      <div className="h-full min-h-[400px] flex items-center justify-center">
        <ErrorState error={error} retry={load} />
      </div>
    );
  }

  if (!topology) return null;

  return (
    <div className="flex flex-col gap-3 h-full min-h-0">
      {/* 顶部工具栏 */}
      <div className="flex items-center justify-between gap-4 p-3 bg-content1 rounded-xl border border-default-200 shrink-0">
        <div className="flex items-center gap-3 flex-wrap">
          <GitFork className="w-5 h-5 text-primary" />
          <Input
            variant="underlined"
            className="max-w-xs"
            value={ivrName}
            onValueChange={setIvrName}
            classNames={{ input: 'text-base font-bold' }}
          />
          <Chip size="sm" variant="flat" color="primary">
            {topology.nodes.length} 节点 · {topology.edges.length} 连线
          </Chip>
          {topology.did && <Chip size="sm" variant="flat" color="primary">DID {topology.did}</Chip>}

          {/* 节点树防呆校验状态 Chip */}
          {validation && (
            <Chip
              size="sm"
              variant="flat"
              color={validation.errors.length > 0 ? 'danger' : validation.warnings.length > 0 ? 'warning' : 'success'}
              startContent={
                validation.errors.length > 0 ? (
                  <AlertCircle className="w-3.5 h-3.5 text-danger" />
                ) : validation.warnings.length > 0 ? (
                  <AlertTriangle className="w-3.5 h-3.5 text-warning" />
                ) : (
                  <CheckCircle2 className="w-3.5 h-3.5 text-success" />
                )
              }
            >
              {validation.errors.length > 0
                ? `${validation.errors.length} 项流程错误`
                : validation.warnings.length > 0
                ? `${validation.warnings.length} 项流程警告`
                : '流程节点树验证通过'}
            </Chip>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="flat"
            startContent={<Play className="w-3.5 h-3.5" />}
            onPress={() => message.info('仿真调试功能即将上线')}
          >
            仿真调试
          </Button>
          <Button
            size="sm"
            color={validation && !validation.valid ? 'default' : 'primary'}
            className="font-bold text-white"
            startContent={<Save className="w-3.5 h-3.5" />}
            onPress={handleSave}
            isLoading={saving}
          >
            保存流程
          </Button>
        </div>
      </div>

      {/* 流程错误/警告提示横幅 */}
      {validation && (validation.errors.length > 0 || validation.warnings.length > 0) && (
        <div className="p-3 bg-content2 rounded-xl border border-default-200 flex flex-col gap-1 text-tiny shrink-0">
          {validation.errors.map((err, idx) => (
            <div key={`err-${idx}`} className="flex items-center gap-1.5 text-danger font-medium">
              <ShieldAlert className="w-3.5 h-3.5 shrink-0" />
              <span>{err}</span>
            </div>
          ))}
          {validation.warnings.map((warn, idx) => (
            <div key={`warn-${idx}`} className="flex items-center gap-1.5 text-warning font-medium">
              <AlertTriangle className="w-3.5 h-3.5 shrink-0" />
              <span>{warn}</span>
            </div>
          ))}
        </div>
      )}

      {/* 三栏布局: 左侧 palette + 中间 canvas + 右侧 inspector */}
      <div className="flex gap-3 flex-1 min-h-0 h-full">
        <NodePalette />
        <div className="flex-1 min-w-0 h-full">
          <IvrCanvas
            flow={topology}
            onChange={setTopology}
            selectedNodeId={selectedNodeId}
            onSelectNode={setSelectedNodeId}
          />
        </div>
        <NodeInspector node={selectedNode} onChange={handleNodeChange} />
      </div>
    </div>
  );
}

