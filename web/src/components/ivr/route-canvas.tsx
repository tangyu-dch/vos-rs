// 路由策略可视化拓扑编排画板 (基于 SVG)
import { useRef, useState, type MouseEvent } from 'react';
import { Chip } from '@heroui/react';
import {
  PhoneCall, Hash, Clock, Shield, Route, PhoneForwarded, GitFork, Users,
  Split, Ban, Plus, Trash2,
} from 'lucide-react';
import {
  ROUTE_NODE_CATALOG, ROUTE_NODE_CATALOG_MAP, genRouteEdgeId, genRouteNodeId,
  type RouteEdge, type RouteNode, type RouteTopology,
} from './route-types';

const ICON_MAP: Record<string, React.ComponentType<{ className?: string }>> = {
  PhoneCall, Hash, Clock, Shield, Route, PhoneForwarded, GitFork, Users, Split, Ban,
};

const NODE_WIDTH = 220;
const NODE_HEIGHT = 88;
const PORT_RADIUS = 7;

function getPortPosition(node: RouteNode, portId: string, portType: 'in' | 'out') {
  const catalog = ROUTE_NODE_CATALOG_MAP[node.type];
  const ports = catalog.defaultPorts.filter((p) => p.type === portType);
  const idx = ports.findIndex((p) => p.id === portId);
  const total = ports.length;
  const slot = total > 1 ? idx / (total - 1) : 0.5;
  const x = portType === 'in' ? node.position.x : node.position.x + NODE_WIDTH;
  const y = node.position.y + 20 + slot * (NODE_HEIGHT - 40);
  return { x, y };
}

function edgePath(src: { x: number; y: number }, dst: { x: number; y: number }): string {
  const dx = Math.abs(dst.x - src.x) * 0.5;
  return `M ${src.x} ${src.y} C ${src.x + dx} ${src.y}, ${dst.x - dx} ${dst.y}, ${dst.x} ${dst.y}`;
}

interface RouteCanvasProps {
  topology: RouteTopology;
  onChange: (t: RouteTopology) => void;
}

interface DragState {
  type: 'node' | 'edge';
  nodeId?: string;
  offsetX?: number;
  offsetY?: number;
  fromPort?: { nodeId: string; portId: string };
  cursor: { x: number; y: number };
}

export function RouteCanvas({ topology, onChange }: RouteCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<DragState | null>(null);

  const toCanvasCoords = (clientX: number, clientY: number) => {
    const svg = svgRef.current;
    if (!svg) return { x: 0, y: 0 };
    const rect = svg.getBoundingClientRect();
    return { x: clientX - rect.left, y: clientY - rect.top };
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const data = e.dataTransfer.getData('application/route-node');
    if (!data) return;
    const entry = JSON.parse(data) as typeof ROUTE_NODE_CATALOG[number];
    const pos = toCanvasCoords(e.clientX, e.clientY);
    const newNode: RouteNode = {
      id: genRouteNodeId(entry.type),
      type: entry.type,
      title: entry.title,
      description: entry.description,
      position: { x: pos.x - NODE_WIDTH / 2, y: pos.y - NODE_HEIGHT / 2 },
      config: { ...entry.defaultConfig },
    };
    onChange({ ...topology, nodes: [...topology.nodes, newNode] });
  };

  const startNodeDrag = (e: MouseEvent, nodeId: string) => {
    e.stopPropagation();
    const node = topology.nodes.find((n) => n.id === nodeId);
    if (!node) return;
    const cursor = toCanvasCoords(e.clientX, e.clientY);
    setDrag({
      type: 'node',
      nodeId,
      offsetX: cursor.x - node.position.x,
      offsetY: cursor.y - node.position.y,
      cursor,
    });
  };

  const startEdgeDrag = (e: MouseEvent, nodeId: string, portId: string) => {
    e.stopPropagation();
    const cursor = toCanvasCoords(e.clientX, e.clientY);
    setDrag({ type: 'edge', fromPort: { nodeId, portId }, cursor });
  };

  const handleMouseMove = (e: MouseEvent) => {
    if (!drag) return;
    const cursor = toCanvasCoords(e.clientX, e.clientY);
    if (drag.type === 'node' && drag.nodeId) {
      onChange({
        ...topology,
        nodes: topology.nodes.map((n) =>
          n.id === drag.nodeId
            ? { ...n, position: { x: cursor.x - (drag.offsetX ?? 0), y: cursor.y - (drag.offsetY ?? 0) } }
            : n
        ),
      });
    } else if (drag.type === 'edge') {
      setDrag({ ...drag, cursor });
    }
  };

  const handleMouseUp = (e: MouseEvent) => {
    if (drag?.type === 'edge' && drag.fromPort) {
      const target = (e.target as SVGElement).closest('[data-node-id]');
      const targetId = target?.getAttribute('data-node-id');
      if (targetId && targetId !== drag.fromPort.nodeId) {
        const newEdge: RouteEdge = {
          id: genRouteEdgeId(),
          source: drag.fromPort.nodeId,
          target: targetId,
          sourcePort: drag.fromPort.portId,
          label: ROUTE_NODE_CATALOG_MAP[
            topology.nodes.find((n) => n.id === drag.fromPort!.nodeId)!.type
          ].defaultPorts.find((p) => p.id === drag.fromPort!.portId)?.label,
        };
        const exists = topology.edges.some(
          (ed) => ed.source === newEdge.source && ed.target === newEdge.target && ed.sourcePort === newEdge.sourcePort
        );
        if (!exists) onChange({ ...topology, edges: [...topology.edges, newEdge] });
      }
    }
    setDrag(null);
  };

  const deleteNode = (nodeId: string, e: MouseEvent) => {
    e.stopPropagation();
    onChange({
      ...topology,
      nodes: topology.nodes.filter((n) => n.id !== nodeId),
      edges: topology.edges.filter((ed) => ed.source !== nodeId && ed.target !== nodeId),
    });
  };

  const deleteEdge = (edgeId: string) => {
    onChange({ ...topology, edges: topology.edges.filter((ed) => ed.id !== edgeId) });
  };

  const renderNode = (node: RouteNode) => {
    const catalog = ROUTE_NODE_CATALOG_MAP[node.type];
    const Icon = ICON_MAP[catalog.icon] ?? Plus;
    const inPorts = catalog.defaultPorts.filter((p) => p.type === 'in');
    const outPorts = catalog.defaultPorts.filter((p) => p.type === 'out');
    return (
      <g
        key={node.id}
        data-node-id={node.id}
        transform={`translate(${node.position.x}, ${node.position.y})`}
        className="cursor-move"
        onMouseDown={(e) => startNodeDrag(e, node.id)}
        onClick={(e) => e.stopPropagation()}
      >
        <rect
          width={NODE_WIDTH}
          height={NODE_HEIGHT}
          rx={10}
          className="fill-content1 stroke-default-200"
          strokeWidth={1}
        />
        <rect width={4} height={NODE_HEIGHT} rx={2} className={`fill-current ${catalog.color.split(' ')[0].replace('/15', '/100')}`} />
        <foreignObject x={12} y={12} width={24} height={24}>
          <div className={`w-6 h-6 rounded-lg flex items-center justify-center ${catalog.color}`}>
            <Icon className="w-3.5 h-3.5" />
          </div>
        </foreignObject>
        <foreignObject x={42} y={10} width={NODE_WIDTH - 80} height={50}>
          <div className="flex flex-col">
            <span className="text-xs font-bold text-foreground truncate">{node.title}</span>
            <span className="text-[10px] text-default-400 line-clamp-2">{node.description}</span>
          </div>
        </foreignObject>
        <foreignObject x={NODE_WIDTH - 28} y={8} width={20} height={20}>
          <button
            type="button"
            className="w-5 h-5 rounded-md flex items-center justify-center text-danger hover:bg-danger/10"
            onClick={(e) => deleteNode(node.id, e as unknown as MouseEvent)}
          >
            <Trash2 className="w-3 h-3" />
          </button>
        </foreignObject>
        {inPorts.map((port) => {
          const pos = getPortPosition(node, port.id, 'in');
          return (
            <circle
              key={`in-${port.id}`}
              cx={pos.x - node.position.x}
              cy={pos.y - node.position.y}
              r={PORT_RADIUS}
              className="fill-default-300 stroke-content1"
              strokeWidth={2}
            />
          );
        })}
        {outPorts.map((port) => {
          const pos = getPortPosition(node, port.id, 'out');
          return (
            <g key={`out-${port.id}`} className="cursor-crosshair" onMouseDown={(e) => startEdgeDrag(e, node.id, port.id)}>
              <circle
                cx={pos.x - node.position.x}
                cy={pos.y - node.position.y}
                r={PORT_RADIUS}
                className="fill-primary stroke-content1 hover:fill-primary/80"
                strokeWidth={2}
              />
              <foreignObject x={pos.x - node.position.x - 30} y={pos.y - node.position.y - 22} width={60} height={16}>
                <span className="text-[9px] text-default-400 text-center block">{port.label}</span>
              </foreignObject>
            </g>
          );
        })}
      </g>
    );
  };

  const renderEdge = (edge: RouteEdge) => {
    const src = topology.nodes.find((n) => n.id === edge.source);
    const dst = topology.nodes.find((n) => n.id === edge.target);
    if (!src || !dst) return null;
    const srcPort = edge.sourcePort ?? ROUTE_NODE_CATALOG_MAP[src.type].defaultPorts.find((p) => p.type === 'out')?.id ?? 'out';
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
          markerEnd="url(#arrow-route)"
        />
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
        {edge.label && (
          <foreignObject x={midX - 20} y={midY - 8} width={40} height={16}>
            <div className="flex justify-center">
              <Chip size="sm" variant="flat" color="secondary" className="text-[9px] h-4">{edge.label}</Chip>
            </div>
          </foreignObject>
        )}
      </g>
    );
  };

  const renderTempEdge = () => {
    if (drag?.type !== 'edge' || !drag.fromPort) return null;
    const src = topology.nodes.find((n) => n.id === drag.fromPort!.nodeId);
    if (!src) return null;
    const srcPos = getPortPosition(src, drag.fromPort.portId, 'out');
    return (
      <path d={edgePath(srcPos, drag.cursor)} fill="none" className="stroke-primary/70" strokeWidth={2} strokeDasharray="4 2" />
    );
  };

  return (
    <div className="flex gap-3 h-full min-h-0">
      {/* 左侧节点工具箱 */}
      <div className="w-64 shrink-0 h-full p-3 bg-content1 rounded-xl border border-default-200 flex flex-col gap-2 overflow-y-auto">
        <div className="flex items-center gap-2 pb-2 border-b border-default-200 shrink-0">
          <Plus className="w-4 h-4 text-primary" />
          <span className="text-xs font-bold">路由节点</span>
        </div>
        {(['source', 'filter', 'action'] as const).map((cat) => {
          const items = ROUTE_NODE_CATALOG.filter((n) => n.category === cat);
          const catLabel = cat === 'source' ? '呼入源' : cat === 'filter' ? '过滤条件' : '执行动作';
          return (
            <div key={cat} className="flex flex-col gap-2">
              <span className="text-[10px] font-semibold text-default-500 uppercase">{catLabel}</span>
              {items.map((entry) => {
                const Icon = ICON_MAP[entry.icon] ?? Plus;
                return (
                  <div
                    key={entry.type}
                    draggable
                    onDragStart={(e) => {
                      e.dataTransfer.setData('application/route-node', JSON.stringify(entry));
                      e.dataTransfer.effectAllowed = 'copy';
                    }}
                    className={`p-2.5 rounded-lg border ${entry.color} cursor-grab active:cursor-grabbing hover:shadow-md transition-all flex items-center gap-2 bg-content1`}
                  >
                    <div className="w-6 h-6 rounded-md flex items-center justify-center bg-content2 shrink-0">
                      <Icon className="w-3 h-3" />
                    </div>
                    <div className="flex flex-col min-w-0">
                      <span className="text-[11px] font-bold truncate">{entry.title}</span>
                      <span className="text-[9px] text-default-400 truncate">{entry.description}</span>
                    </div>
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>

      {/* 中间画布 */}
      <div className="flex-1 min-w-0 relative">
        <svg
          ref={svgRef}
          className="w-full h-full bg-background rounded-xl border-2 border-dashed border-default-200"
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDrop}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={() => setDrag(null)}
        >
          <defs>
            <marker id="arrow-route" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth">
              <path d="M0,0 L0,6 L8,3 z" className="fill-primary/70" />
            </marker>
          </defs>
          <pattern id="grid-route" width="20" height="20" patternUnits="userSpaceOnUse">
            <path d="M 20 0 L 0 0 0 20" fill="none" className="stroke-default-100" strokeWidth="0.5" />
          </pattern>
          <rect width="100%" height="100%" fill="url(#grid-route)" />
          {topology.edges.map(renderEdge)}
          {renderTempEdge()}
          {topology.nodes.map(renderNode)}
        </svg>
        {topology.nodes.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
            <div className="text-center">
              <Plus className="w-10 h-10 text-default-300 mx-auto mb-2" />
              <p className="text-sm text-default-400">从左侧拖入节点开始编排路由拓扑</p>
            </div>
          </div>
        )}
        <div className="absolute top-3 right-3">
          <Chip size="sm" variant="flat" color="secondary">
            {topology.nodes.length} 节点 · {topology.edges.length} 连线
          </Chip>
        </div>
      </div>
    </div>
  );
}

// 默认拓扑 (用于初始化)
export function createDefaultTopology(): RouteTopology {
  const inboundId = 'demo-inbound';
  const timeFilterId = 'demo-time-filter';
  const queueId = 'demo-queue';
  const ivrId = 'demo-ivr';
  return {
    id: `topo-${Date.now().toString(36)}`,
    name: '默认路由拓扑',
    enabled: true,
    nodes: [
      {
        id: inboundId,
        type: 'inbound',
        title: '呼入源',
        description: '所有 DID 入呼',
        position: { x: 80, y: 280 },
        config: { source_type: 'did', did: '', trunk_id: '' },
      },
      {
        id: timeFilterId,
        type: 'time_filter',
        title: '时间路由',
        description: '工作时间 vs 非工作时间',
        position: { x: 380, y: 280 },
        config: { time_start: '09:00', time_end: '18:00', weekdays: [1, 2, 3, 4, 5], timezone: 'Asia/Shanghai' },
      },
      {
        id: queueId,
        type: 'queue_branch',
        title: '工作时间转坐席',
        description: '工作时段转人工',
        position: { x: 700, y: 180 },
        config: { queue_id: 'queue-support', priority: 5 },
      },
      {
        id: ivrId,
        type: 'ivr_branch',
        title: '非工作时间转 IVR',
        description: '非工作时段进 IVR 自助',
        position: { x: 700, y: 380 },
        config: { ivr_id: 'ivr-main' },
      },
    ],
    edges: [
      { id: genRouteEdgeId(), source: inboundId, target: timeFilterId, sourcePort: 'out', label: '进入' },
      { id: genRouteEdgeId(), source: timeFilterId, target: queueId, sourcePort: 'in_window', label: '时段内' },
      { id: genRouteEdgeId(), source: timeFilterId, target: ivrId, sourcePort: 'out_window', label: '时段外' },
    ],
  };
}
