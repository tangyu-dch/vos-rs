import { useRef, useState, useCallback, type MouseEvent } from 'react';
import { Button, Chip } from '@heroui/react';
import {
  Play, Volume2, MessageSquare, Hash, GitBranch, GitFork, Route, Users,
  PhoneCall, PhoneForwarded, Voicemail, Mic, Webhook, Variable,
  AudioLines, Sparkles, Repeat, PhoneOff, Trash2, Plus,
} from 'lucide-react';
import {
  NODE_CATALOG, NODE_CATALOG_MAP, genEdgeId, genNodeId,
  type IvrEdge, type IvrFlow, type IvrNode, type IvrNodeType, type NodeCatalogEntry,
} from './types';

const ICON_MAP: Record<string, React.ComponentType<{ className?: string }>> = {
  Play, Volume2, MessageSquare, Hash, GitBranch, GitFork, Route, Users,
  PhoneCall, PhoneForwarded, Voicemail, Mic, Webhook, Variable,
  AudioLines, Sparkles, Repeat, PhoneOff,
};

const NODE_WIDTH = 220;
const NODE_HEIGHT = 96;
const PORT_RADIUS = 8;

interface CanvasProps {
  flow: IvrFlow;
  onChange: (flow: IvrFlow) => void;
  selectedNodeId: string | null;
  onSelectNode: (id: string | null) => void;
}

interface DragState {
  type: 'node' | 'palette' | 'edge';
  nodeId?: string;
  offsetX?: number;
  offsetY?: number;
  paletteEntry?: NodeCatalogEntry;
  fromPort?: { nodeId: string; portId: string };
  cursor: { x: number; y: number };
}

// 计算端口在画布中的绝对位置
function getPortPosition(node: IvrNode, portId: string, portType: 'in' | 'out') {
  const catalog = NODE_CATALOG_MAP[node.type];
  const ports = catalog.defaultPorts.filter((p) => p.type === portType);
  const idx = ports.findIndex((p) => p.id === portId);
  const total = ports.length;
  const slot = total > 1 ? idx / (total - 1) : 0.5;
  // 输入端口在左侧, 输出端口在右侧
  const x = portType === 'in' ? node.position.x : node.position.x + NODE_WIDTH;
  // 端口在节点上下范围内均匀分布
  const y = node.position.y + 20 + slot * (NODE_HEIGHT - 40);
  return { x, y };
}

// 贝塞尔曲线路径
function edgePath(src: { x: number; y: number }, dst: { x: number; y: number }): string {
  const dx = Math.abs(dst.x - src.x) * 0.5;
  const c1x = src.x + dx;
  const c2x = dst.x - dx;
  return `M ${src.x} ${src.y} C ${c1x} ${src.y}, ${c2x} ${dst.y}, ${dst.x} ${dst.y}`;
}

export function IvrCanvas({ flow, onChange, selectedNodeId, onSelectNode }: CanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<DragState | null>(null);

  // 屏幕坐标 → 画布坐标
  const toCanvasCoords = useCallback((clientX: number, clientY: number) => {
    const svg = svgRef.current;
    if (!svg) return { x: 0, y: 0 };
    const rect = svg.getBoundingClientRect();
    return { x: clientX - rect.left, y: clientY - rect.top };
  }, []);

  // 处理从 palette 拖入新节点
  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const data = e.dataTransfer.getData('application/ivr-node');
    if (!data) return;
    const entry: NodeCatalogEntry = JSON.parse(data);
    const pos = toCanvasCoords(e.clientX, e.clientY);
    const newNode: IvrNode = {
      id: genNodeId(entry.type),
      type: entry.type,
      title: entry.title,
      description: entry.description,
      position: { x: pos.x - NODE_WIDTH / 2, y: pos.y - NODE_HEIGHT / 2 },
      config: { ...entry.defaultConfig },
    };
    onChange({ ...flow, nodes: [...flow.nodes, newNode] });
    onSelectNode(newNode.id);
  };

  // 开始拖拽已有节点
  const startNodeDrag = (e: MouseEvent, nodeId: string) => {
    e.stopPropagation();
    const node = flow.nodes.find((n) => n.id === nodeId);
    if (!node) return;
    const cursor = toCanvasCoords(e.clientX, e.clientY);
    setDrag({
      type: 'node',
      nodeId,
      offsetX: cursor.x - node.position.x,
      offsetY: cursor.y - node.position.y,
      cursor,
    });
    onSelectNode(nodeId);
  };

  // 开始从端口拉出一条连线
  const startEdgeDrag = (e: MouseEvent, nodeId: string, portId: string) => {
    e.stopPropagation();
    const cursor = toCanvasCoords(e.clientX, e.clientY);
    setDrag({
      type: 'edge',
      fromPort: { nodeId, portId },
      cursor,
    });
  };

  // 鼠标移动
  const handleMouseMove = (e: MouseEvent) => {
    if (!drag) return;
    const cursor = toCanvasCoords(e.clientX, e.clientY);
    if (drag.type === 'node' && drag.nodeId) {
      const newX = cursor.x - (drag.offsetX ?? 0);
      const newY = cursor.y - (drag.offsetY ?? 0);
      onChange({
        ...flow,
        nodes: flow.nodes.map((n) =>
          n.id === drag.nodeId ? { ...n, position: { x: newX, y: newY } } : n
        ),
      });
    } else if (drag.type === 'edge') {
      setDrag({ ...drag, cursor });
    }
  };

  // 释放鼠标
  const handleMouseUp = (e: MouseEvent) => {
    if (drag?.type === 'edge' && drag.fromPort) {
      // 检查是否释放到某个节点上 (通过事件 target 反查)
      const target = (e.target as SVGElement).closest('[data-node-id]');
      const targetId = target?.getAttribute('data-node-id');
      if (targetId && targetId !== drag.fromPort.nodeId) {
        const newEdge: IvrEdge = {
          id: genEdgeId(),
          source: drag.fromPort.nodeId,
          target: targetId,
          sourcePort: drag.fromPort.portId,
          label: NODE_CATALOG_MAP[
            flow.nodes.find((n) => n.id === drag.fromPort!.nodeId)!.type
          ].defaultPorts.find((p) => p.id === drag.fromPort!.portId)?.label,
        };
        // 避免重复连线
        const exists = flow.edges.some(
          (ed) =>
            ed.source === newEdge.source &&
            ed.target === newEdge.target &&
            ed.sourcePort === newEdge.sourcePort
        );
        if (!exists) {
          onChange({ ...flow, edges: [...flow.edges, newEdge] });
        }
      }
    }
    setDrag(null);
  };

  // 删除节点 (同时删除关联的边)
  const deleteNode = (nodeId: string, e: MouseEvent) => {
    e.stopPropagation();
    onChange({
      ...flow,
      nodes: flow.nodes.filter((n) => n.id !== nodeId),
      edges: flow.edges.filter((ed) => ed.source !== nodeId && ed.target !== nodeId),
    });
    if (selectedNodeId === nodeId) onSelectNode(null);
  };

  // 删除边
  const deleteEdge = (edgeId: string) => {
    onChange({ ...flow, edges: flow.edges.filter((ed) => ed.id !== edgeId) });
  };

  // 渲染单个节点
  const renderNode = (node: IvrNode) => {
    const catalog = NODE_CATALOG_MAP[node.type];
    const Icon = ICON_MAP[catalog.icon] ?? Plus;
    const isSelected = node.id === selectedNodeId;
    const inPorts = catalog.defaultPorts.filter((p) => p.type === 'in');
    const outPorts = catalog.defaultPorts.filter((p) => p.type === 'out');

    return (
      <g
        key={node.id}
        data-node-id={node.id}
        transform={`translate(${node.position.x}, ${node.position.y})`}
        className="cursor-move"
        onMouseDown={(e) => startNodeDrag(e, node.id)}
        onClick={(e) => {
          e.stopPropagation();
          onSelectNode(node.id);
        }}
      >
        {/* 节点矩形 */}
        <rect
          width={NODE_WIDTH}
          height={NODE_HEIGHT}
          rx={10}
          className={isSelected ? 'fill-content1 stroke-primary' : 'fill-content1 stroke-default-200'}
          strokeWidth={isSelected ? 2 : 1}
        />
        {/* 左侧色条 */}
        <rect width={4} height={NODE_HEIGHT} rx={2} className={`fill-current ${catalog.color.split(' ')[0].replace('/15', '/100')}`} />
        {/* 图标 */}
        <foreignObject x={12} y={12} width={24} height={24}>
          <div className={`w-6 h-6 rounded-lg flex items-center justify-center ${catalog.color}`}>
            <Icon className="w-3.5 h-3.5" />
          </div>
        </foreignObject>
        {/* 标题 */}
        <foreignObject x={42} y={10} width={NODE_WIDTH - 80} height={50}>
          <div className="flex flex-col">
            <span className="text-xs font-bold text-foreground truncate">{node.title}</span>
            <span className="text-[10px] text-default-400 line-clamp-2">{node.description}</span>
          </div>
        </foreignObject>
        {/* 删除按钮 */}
        <foreignObject x={NODE_WIDTH - 28} y={8} width={20} height={20}>
          <button
            type="button"
            className="w-5 h-5 rounded-md flex items-center justify-center text-danger hover:bg-danger/10"
            onClick={(e) => deleteNode(node.id, e as unknown as MouseEvent)}
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </foreignObject>
        {/* 输入端口 */}
        {inPorts.map((port) => {
          const pos = getPortPosition(node, port.id, 'in');
          return (
            <g key={`in-${port.id}`}>
              <circle
                cx={pos.x - node.position.x}
                cy={pos.y - node.position.y}
                r={PORT_RADIUS}
                className="fill-default-300 stroke-content1"
                strokeWidth={2}
              />
            </g>
          );
        })}
        {/* 输出端口 (可拖拽) */}
        {outPorts.map((port) => {
          const pos = getPortPosition(node, port.id, 'out');
          return (
            <g
              key={`out-${port.id}`}
              className="cursor-crosshair"
              onMouseDown={(e) => startEdgeDrag(e, node.id, port.id)}
            >
              <circle
                cx={pos.x - node.position.x}
                cy={pos.y - node.position.y}
                r={PORT_RADIUS}
                className="fill-primary stroke-content1 hover:fill-primary/80"
                strokeWidth={2}
              />
              <foreignObject
                x={pos.x - node.position.x - 30}
                y={pos.y - node.position.y - 22}
                width={60}
                height={16}
              >
                <span className="text-[9px] text-default-400 text-center block">{port.label}</span>
              </foreignObject>
            </g>
          );
        })}
      </g>
    );
  };

  // 渲染一条边
  const renderEdge = (edge: IvrEdge) => {
    const src = flow.nodes.find((n) => n.id === edge.source);
    const dst = flow.nodes.find((n) => n.id === edge.target);
    if (!src || !dst) return null;
    const srcPort = edge.sourcePort ?? NODE_CATALOG_MAP[src.type].defaultPorts.find((p) => p.type === 'out')?.id ?? 'out';
    const srcPos = getPortPosition(src, srcPort, 'out');
    const dstPos = getPortPosition(dst, 'in', 'in');
    const midX = (srcPos.x + dstPos.x) / 2;
    const midY = (srcPos.y + dstPos.y) / 2;
    return (
      <g key={edge.id} className="group">
        <path
          d={edgePath(srcPos, dstPos)}
          fill="none"
          className="stroke-primary/70 group-hover:stroke-primary"
          strokeWidth={2}
          markerEnd="url(#arrow)"
        />
        {/* 删除按钮 (悬停显示) */}
        <g
          transform={`translate(${midX - 10}, ${midY - 10})`}
          className="opacity-0 group-hover:opacity-100 cursor-pointer"
          onClick={() => deleteEdge(edge.id)}
        >
          <circle cx={10} cy={10} r={9} className="fill-danger" />
          <foreignObject x={3} y={3} width={14} height={14}>
            <Trash2 className="w-3 h-3 text-white" />
          </foreignObject>
        </g>
        {/* 边标签 */}
        {edge.label && (
          <foreignObject x={midX - 20} y={midY - 8} width={40} height={16}>
            <div className="flex justify-center">
              <Chip size="sm" variant="flat" color="secondary" className="text-[9px] h-4">
                {edge.label}
              </Chip>
            </div>
          </foreignObject>
        )}
      </g>
    );
  };

  // 渲染拖拽中的临时边
  const renderTempEdge = () => {
    if (drag?.type !== 'edge' || !drag.fromPort) return null;
    const src = flow.nodes.find((n) => n.id === drag.fromPort!.nodeId);
    if (!src) return null;
    const srcPos = getPortPosition(src, drag.fromPort.portId, 'out');
    return (
      <path
        d={edgePath(srcPos, drag.cursor)}
        fill="none"
        className="stroke-primary/70"
        strokeWidth={2}
        strokeDasharray="4 2"
      />
    );
  };

  return (
    <div className="relative w-full h-full">
      <svg
        ref={svgRef}
        className="w-full h-full bg-background rounded-xl border-2 border-dashed border-default-200"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={() => setDrag(null)}
        onClick={() => onSelectNode(null)}
      >
        <defs>
          <marker
            id="arrow"
            markerWidth="10"
            markerHeight="10"
            refX="8"
            refY="3"
            orient="auto"
            markerUnits="strokeWidth"
          >
            <path d="M0,0 L0,6 L8,3 z" className="fill-primary/70" />
          </marker>
        </defs>
        {/* 网格背景 */}
        <pattern id="grid" width="20" height="20" patternUnits="userSpaceOnUse">
          <path d="M 20 0 L 0 0 0 20" fill="none" className="stroke-default-100" strokeWidth="0.5" />
        </pattern>
        <rect width="100%" height="100%" fill="url(#grid)" />
        {/* 渲染所有边 */}
        {flow.edges.map(renderEdge)}
        {renderTempEdge()}
        {/* 渲染所有节点 */}
        {flow.nodes.map(renderNode)}
      </svg>
      {/* 顶部工具栏 */}
      <div className="absolute top-3 right-3 flex gap-2">
        <Button
          size="sm"
          variant="flat"
          startContent={<Plus className="w-3.5 h-3.5" />}
          onPress={() => {
            const entry = NODE_CATALOG_MAP['prompt'];
            const newNode: IvrNode = {
              id: genNodeId('prompt'),
              type: 'prompt',
              title: entry.title,
              description: entry.description,
              position: { x: 100 + Math.random() * 200, y: 100 + Math.random() * 200 },
              config: { ...entry.defaultConfig },
            };
            onChange({ ...flow, nodes: [...flow.nodes, newNode] });
          }}
        >
          快速添加节点
        </Button>
      </div>
      {/* 空画布提示 */}
      {flow.nodes.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div className="text-center">
            <Plus className="w-10 h-10 text-default-300 mx-auto mb-2" />
            <p className="text-sm text-default-400">从左侧拖入节点到此处开始编排</p>
          </div>
        </div>
      )}
    </div>
  );
}

// 节点工具箱 (左侧)
export function NodePalette() {
  const categories: Array<{ key: string; label: string }> = [
    { key: 'flow', label: '流程控制' },
    { key: 'media', label: '媒体处理' },
    { key: 'routing', label: '路由转接' },
    { key: 'integration', label: '集成能力' },
    { key: 'system', label: '系统' },
  ];

  return (
    <div className="w-72 shrink-0 h-full p-4 bg-content1 rounded-xl border border-default-200 flex flex-col gap-3 overflow-y-auto">
      <div className="flex items-center gap-2 pb-2 border-b border-default-200 shrink-0">
        <Plus className="w-4 h-4 text-primary" />
        <span className="text-xs font-bold">节点工具箱</span>
      </div>
      <p className="text-[10px] text-default-400">按住下方卡片拖入画布即可创建节点</p>
      {categories.map((cat) => {
        const items = NODE_CATALOG.filter((n) => n.category === cat.key);
        if (items.length === 0) return null;
        return (
          <div key={cat.key} className="flex flex-col gap-2">
            <span className="text-[10px] font-semibold text-default-500 uppercase tracking-wide">
              {cat.label}
            </span>
            {items.map((entry) => {
              const Icon = ICON_MAP[entry.icon] ?? Plus;
              return (
                <div
                  key={entry.type}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData('application/ivr-node', JSON.stringify(entry));
                    e.dataTransfer.effectAllowed = 'copy';
                  }}
                  className={`p-3 rounded-lg border ${entry.color} cursor-grab active:cursor-grabbing hover:shadow-md transition-all flex items-center gap-3 bg-content1`}
                >
                  <div className="w-7 h-7 rounded-lg flex items-center justify-center bg-content2 shrink-0">
                    <Icon className="w-3.5 h-3.5" />
                  </div>
                  <div className="flex flex-col min-w-0">
                    <span className="text-xs font-bold truncate">{entry.title}</span>
                    <span className="text-[10px] text-default-400 truncate">{entry.description}</span>
                  </div>
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}

// 属性面板 (右侧)
interface InspectorProps {
  node: IvrNode | null;
  onChange: (node: IvrNode) => void;
}

export function NodeInspector({ node, onChange }: InspectorProps) {
  if (!node) {
    return (
      <div className="w-80 shrink-0 h-full p-4 bg-content1 rounded-xl border border-default-200 flex items-center justify-center">
        <div className="text-center">
          <Sparkles className="w-8 h-8 text-default-300 mx-auto mb-2" />
          <p className="text-xs text-default-400">在画布中点击选中节点查看属性</p>
        </div>
      </div>
    );
  }
  const catalog = NODE_CATALOG_MAP[node.type];
  const Icon = ICON_MAP[catalog.icon] ?? Plus;
  return (
    <div className="w-80 shrink-0 h-full p-4 bg-content1 rounded-xl border border-default-200 flex flex-col gap-3 overflow-y-auto">
      <div className="flex items-center gap-2 pb-2 border-b border-default-200 shrink-0">
        <Sparkles className="w-4 h-4 text-primary" />
        <span className="text-xs font-bold">节点属性</span>
      </div>
      <div className={`p-3 rounded-lg border ${catalog.color} flex items-center gap-2`}>
        <div className="w-8 h-8 rounded-lg flex items-center justify-center bg-content2">
          <Icon className="w-4 h-4" />
        </div>
        <div className="flex flex-col">
          <span className="text-xs font-bold">{node.title}</span>
          <span className="text-[10px] text-default-400">ID: {node.id}</span>
        </div>
      </div>
      <div className="flex flex-col gap-2">
        <label className="text-xs font-semibold">节点标题</label>
        <input
          className="text-xs px-3 py-2 rounded-lg border border-default-200 bg-content2"
          value={node.title}
          onChange={(e) => onChange({ ...node, title: e.target.value })}
        />
      </div>
      <div className="flex flex-col gap-2">
        <label className="text-xs font-semibold">节点描述</label>
        <textarea
          className="text-xs px-3 py-2 rounded-lg border border-default-200 bg-content2 min-h-16"
          value={node.description ?? ''}
          onChange={(e) => onChange({ ...node, description: e.target.value })}
        />
      </div>
      <div className="flex flex-col gap-2 pt-2 border-t border-default-200">
        <span className="text-xs font-semibold">配置参数</span>
        <NodeConfigFormWrapper node={node} onChange={onChange} />
      </div>
    </div>
  );
}

// 包装 NodeConfigForm 以保持文件解耦
import { NodeConfigForm } from './ivr-node-forms';
function NodeConfigFormWrapper({ node, onChange }: { node: IvrNode; onChange: (n: IvrNode) => void }) {
  return (
    <NodeConfigForm
      type={node.type}
      config={node.config}
      onChange={(config) => onChange({ ...node, config })}
    />
  );
}

// 重新导出节点类型供外部使用
export type { IvrFlow, IvrNode, IvrEdge, IvrNodeType };
