import { useRef, useState, useCallback, type MouseEvent, type WheelEvent } from 'react';
import { Button, Chip, Tooltip } from '@heroui/react';
import {
  PhoneCall, Hash, Clock, Shield, Route, PhoneForwarded, GitFork, Users,
  Split, Ban, Plus, Trash2, Sparkles, ZoomIn, ZoomOut, Maximize2, LayoutGrid,
} from 'lucide-react';
import {
  ROUTE_NODE_CATALOG, ROUTE_NODE_CATALOG_MAP, genRouteEdgeId, genRouteNodeId,
  type RouteEdge, type RouteNode, type RouteTopology, type RouteNodeCatalogEntry,
} from './route-types';
import { autoLayoutNodes } from './ivr-canvas';
export { autoLayoutNodes };

const ICON_MAP: Record<string, React.ComponentType<{ className?: string }>> = {
  PhoneCall, Hash, Clock, Shield, Route, PhoneForwarded, GitFork, Users, Split, Ban,
};

const NODE_W = 220;
const PORT_R = 6;

/** 根据 catalog 颜色决定节点的边框与顶栏半透明渲染色 */
function getNodeTheme(colorClass: string) {
  if (colorClass.includes('text-success') || colorClass.includes('border-success')) {
    return { stroke: 'stroke-success/70', strokeSelected: 'stroke-success', fillHeader: 'fill-success/15', ring: 'ring-success/30' };
  }
  if (colorClass.includes('text-warning') || colorClass.includes('border-warning')) {
    return { stroke: 'stroke-warning/70', strokeSelected: 'stroke-warning', fillHeader: 'fill-warning/15', ring: 'ring-warning/30' };
  }
  if (colorClass.includes('text-danger') || colorClass.includes('border-danger')) {
    return { stroke: 'stroke-danger/70', strokeSelected: 'stroke-danger', fillHeader: 'fill-danger/15', ring: 'ring-danger/30' };
  }
  if (colorClass.includes('text-secondary') || colorClass.includes('border-secondary')) {
    return { stroke: 'stroke-secondary/70', strokeSelected: 'stroke-secondary', fillHeader: 'fill-secondary/15', ring: 'ring-secondary/30' };
  }
  return { stroke: 'stroke-primary/70', strokeSelected: 'stroke-primary', fillHeader: 'fill-primary/15', ring: 'ring-primary/30' };
}

export function getRouteNodePorts(node: RouteNode): {
  inPorts: Array<{ id: string; label: string; type: 'in' | 'out' }>;
  outPorts: Array<{ id: string; label: string; type: 'in' | 'out' }>;
} {
  const cat = ROUTE_NODE_CATALOG_MAP[node.type];
  if (!cat) return { inPorts: [{ id: 'in', label: '呼入', type: 'in' }], outPorts: [{ id: 'out', label: '出口', type: 'out' }] };

  let inPorts = cat.defaultPorts.filter((p) => p.type === 'in');
  if (inPorts.length === 0) inPorts = [{ id: 'in', label: '呼入', type: 'in' }];

  let outPorts: Array<{ id: string; label: string; type: 'in' | 'out' }> = [];

  if (node.type === 'prefix_match') {
    const prefixes = Array.isArray(node.config?.prefixes)
      ? (node.config.prefixes as Array<{ prefix: string; label?: string }>)
      : [];
    if (prefixes.length > 0) {
      outPorts = prefixes.map((item) => ({
        id: `prefix-${item.prefix}`,
        label: `前缀 ${item.prefix}${item.label ? `: ${item.label}` : ''}`,
        type: 'out',
      }));
      outPorts.push({ id: 'nomatch', label: '默认/不匹配', type: 'out' });
    }
  }

  if (outPorts.length === 0) {
    outPorts = [{ id: 'out', label: '出口', type: 'out' }];
  }

  return { inPorts, outPorts };
}

export function getRouteNodeDimensions(node: RouteNode) {
  const { outPorts } = getRouteNodePorts(node);
  const width = NODE_W;
  const height = outPorts.length > 1 ? 46 + outPorts.length * 28 + 8 : 58;
  return { width, height };
}

/** 计算端口在节点内的相对坐标 */
function portOffset(node: RouteNode, portId: string, portType: 'in' | 'out') {
  const { inPorts, outPorts } = getRouteNodePorts(node);
  const { width, height } = getRouteNodeDimensions(node);

  if (portType === 'in') {
    const idx = inPorts.findIndex((p) => p.id === portId);
    const total = inPorts.length;
    const y = total > 1 ? 20 + (idx / (total - 1)) * (height - 40) : height / 2;
    return { x: 0, y };
  } else {
    if (outPorts.length <= 1) {
      return { x: width, y: height / 2 };
    }
    const idx = outPorts.findIndex((p) => p.id === portId);
    const y = 46 + (idx >= 0 ? idx : 0) * 28 + 14;
    return { x: width, y };
  }
}

/** 贝塞尔曲线路径 */
function bezier(src: { x: number; y: number }, dst: { x: number; y: number }) {
  const dx = Math.abs(dst.x - src.x) * 0.45;
  return `M ${src.x} ${src.y} C ${src.x + dx} ${src.y}, ${dst.x - dx} ${dst.y}, ${dst.x} ${dst.y}`;
}

interface RouteCanvasProps {
  topology: RouteTopology;
  onChange: (t: RouteTopology) => void;
  selectedNodeId?: string | null;
  onSelectNode?: (id: string | null) => void;
}

interface DragState {
  type: 'node' | 'edge' | 'pan';
  nodeId?: string;
  offsetX?: number;
  offsetY?: number;
  fromPort?: { nodeId: string; portId: string };
  cursor: { x: number; y: number };
}

export function RouteCanvas({ topology, onChange, selectedNodeId = null, onSelectNode }: RouteCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [localSelected, setLocalSelected] = useState<string | null>(null);
  const selectedId = selectedNodeId ?? localSelected;

  // 平移与缩放状态
  const [zoom, setZoom] = useState(1.0);
  const [pan, setPan] = useState({ x: 0, y: 0 });

  const fireSelect = useCallback((id: string | null) => {
    setLocalSelected(id);
    onSelectNode?.(id);
  }, [onSelectNode]);

  const toCanvas = useCallback(
    (cx: number, cy: number) => {
      const svg = svgRef.current;
      if (!svg) return { x: 0, y: 0 };
      const rect = svg.getBoundingClientRect();
      const rawX = cx - rect.left;
      const rawY = cy - rect.top;
      return {
        x: (rawX - pan.x) / zoom,
        y: (rawY - pan.y) / zoom,
      };
    },
    [pan.x, pan.y, zoom]
  );

  const handleWheel = (e: WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? -0.08 : 0.08;
    setZoom((z) => Math.max(0.4, Math.min(2.0, Number((z + delta).toFixed(2)))));
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const data = e.dataTransfer.getData('application/route-node');
    if (!data) return;
    const entry: RouteNodeCatalogEntry = JSON.parse(data);
    const pos = toCanvas(e.clientX, e.clientY);
    const newNode: RouteNode = {
      id: genRouteNodeId(entry.type),
      type: entry.type,
      title: entry.title,
      description: entry.description,
      position: { x: pos.x - NODE_W / 2, y: pos.y - 42 },
      config: { ...entry.defaultConfig },
    };
    onChange({ ...topology, nodes: [...topology.nodes, newNode] });
    fireSelect(newNode.id);
  };

  const startCanvasPan = (e: MouseEvent) => {
    if (e.target === svgRef.current || (e.target as HTMLElement).tagName === 'rect') {
      setDrag({
        type: 'pan',
        cursor: { x: e.clientX - pan.x, y: e.clientY - pan.y },
      });
    }
  };

  const startNodeDrag = (e: MouseEvent, nodeId: string) => {
    e.stopPropagation();
    const node = topology.nodes.find((n) => n.id === nodeId);
    if (!node) return;
    const cursor = toCanvas(e.clientX, e.clientY);
    setDrag({ type: 'node', nodeId, offsetX: cursor.x - node.position.x, offsetY: cursor.y - node.position.y, cursor });
    fireSelect(nodeId);
  };

  const startEdgeDrag = (e: MouseEvent, nodeId: string, portId: string) => {
    e.stopPropagation();
    setDrag({ type: 'edge', fromPort: { nodeId, portId }, cursor: toCanvas(e.clientX, e.clientY) });
  };

  const handleMouseMove = (e: MouseEvent) => {
    if (!drag) return;
    if (drag.type === 'pan') {
      setPan({
        x: e.clientX - drag.cursor.x,
        y: e.clientY - drag.cursor.y,
      });
      return;
    }
    const cursor = toCanvas(e.clientX, e.clientY);
    if (drag.type === 'node' && drag.nodeId) {
      onChange({
        ...topology,
        nodes: topology.nodes.map((n) =>
          n.id === drag.nodeId ? { ...n, position: { x: cursor.x - (drag.offsetX ?? 0), y: cursor.y - (drag.offsetY ?? 0) } } : n,
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
        const srcNode = topology.nodes.find((n) => n.id === drag.fromPort!.nodeId)!;
        const { outPorts } = getRouteNodePorts(srcNode);
        const portObj = outPorts.find((p) => p.id === drag.fromPort!.portId);
        const portLabel = portObj?.label ?? '连线';
        const newEdge: RouteEdge = { id: genRouteEdgeId(), source: drag.fromPort.nodeId, target: targetId, sourcePort: drag.fromPort.portId, label: portLabel };
        const exists = topology.edges.some((ed) => ed.source === newEdge.source && ed.target === newEdge.target && ed.sourcePort === newEdge.sourcePort);
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
    if (selectedId === nodeId) fireSelect(null);
  };

  const deleteEdge = (edgeId: string) => {
    onChange({ ...topology, edges: topology.edges.filter((ed) => ed.id !== edgeId) });
  };

  const handleAutoLayout = () => {
    const layouted = autoLayoutNodes(topology.nodes, topology.edges);
    onChange({ ...topology, nodes: layouted });
    setPan({ x: 0, y: 0 });
    setZoom(1.0);
  };

  /** 渲染单个节点 */
  const renderNode = (node: RouteNode) => {
    const cat = ROUTE_NODE_CATALOG_MAP[node.type];
    const Icon = ICON_MAP[cat.icon] ?? Plus;
    const selected = node.id === selectedId;
    const { inPorts, outPorts } = getRouteNodePorts(node);
    const { width: NODE_W, height: NODE_H } = getRouteNodeDimensions(node);
    const theme = getNodeTheme(cat.color);

    return (
      <g
        key={node.id}
        data-node-id={node.id}
        transform={`translate(${node.position.x}, ${node.position.y})`}
        className="topo-node cursor-move select-none"
        onMouseDown={(e) => startNodeDrag(e, node.id)}
        onClick={(e) => { e.stopPropagation(); fireSelect(node.id); }}
      >
        {/* 阴影底板 */}
        <rect width={NODE_W} height={NODE_H} y={4} rx={14} className="fill-black/30 blur-sm" opacity={0.5} />

        {/* 节点主体背景卡片（边框颜色与工具箱一致，去掉左边白边） */}
        <rect
          width={NODE_W} height={NODE_H} rx={14}
          className={`fill-content1 ${selected ? `${theme.strokeSelected} ring-2 ${theme.ring} shadow-2xl` : `${theme.stroke} hover:stroke-foreground/60`}`}
          strokeWidth={selected ? 2 : 1.5}
        />

        {/* 顶栏主题颜色填充 */}
        <rect width={NODE_W} height={42} rx={14} className={theme.fillHeader} />
        <line x1={0} y1={42} x2={NODE_W} y2={42} className="stroke-default-200/30" strokeWidth={1} />

        {/* 顶部标题区（图标 + 名称 + 类型 + 删除） */}
        <foreignObject x={10} y={7} width={28} height={28}>
          <div className={`w-7 h-7 rounded-lg flex items-center justify-center ${cat.color} shadow-xs`}>
            <Icon className="w-4 h-4" />
          </div>
        </foreignObject>

        <foreignObject x={44} y={6} width={NODE_W - 76} height={34}>
          <div className="flex flex-col">
            <span className="text-xs font-bold text-foreground truncate leading-tight">{node.title}</span>
            <span className="text-[9px] text-default-400 truncate leading-tight mt-0.5">{node.description || node.type}</span>
          </div>
        </foreignObject>

        {/* 删除按钮（hover 显示） */}
        <foreignObject x={NODE_W - 26} y={6} width={18} height={18}>
          <button
            type="button"
            className="w-4.5 h-4.5 rounded-md flex items-center justify-center text-danger/80 hover:text-danger hover:bg-danger/10 transition-colors"
            onClick={(e) => deleteNode(node.id, e as unknown as MouseEvent)}
            aria-label="删除节点"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </foreignObject>

        {/* 多端口选项行列表渲染（分流分支/过滤规则独立渲染） */}
        {outPorts.length > 1 && (
          <g transform="translate(0, 46)">
            {outPorts.map((port, idx) => {
              const rowY = idx * 28;
              return (
                <g key={`row-${port.id}`} transform={`translate(0, ${rowY})`}>
                  {/* 选项背景条 */}
                  <rect x={6} y={2} width={NODE_W - 12} height={24} rx={6} className="fill-default-50/50 hover:fill-default-100/60 transition-colors" />

                  {/* 选项名称 Badge */}
                  <foreignObject x={12} y={5} width={NODE_W - 40} height={18}>
                    <div className="flex items-center gap-1.5 min-w-0">
                      <span className="w-1.5 h-1.5 rounded-full bg-primary/70 shrink-0" />
                      <span className="text-[10px] font-medium text-foreground truncate">{port.label}</span>
                    </div>
                  </foreignObject>
                </g>
              );
            })}
          </g>
        )}

        {/* 输入端口 */}
        {inPorts.map((port) => {
          const { x, y } = portOffset(node, port.id, 'in');
          return (
            <g key={`in-${port.id}`}>
              <circle cx={x} cy={y} r={PORT_R + 2} className="fill-content1 stroke-default-300" strokeWidth={1.5} />
              <circle cx={x} cy={y} r={PORT_R - 2} className="fill-default-400" />
            </g>
          );
        })}

        {/* 输出端口（独立可拖拽连线锚点，按分支区分，不再挤压） */}
        {outPorts.map((port) => {
          const { x, y } = portOffset(node, port.id, 'out');
          return (
            <g
              key={`out-${port.id}`}
              className="cursor-crosshair group/port"
              onMouseDown={(e) => startEdgeDrag(e, node.id, port.id)}
            >
              {/* 放大热区响应点击与拖拽 */}
              <circle cx={x} cy={y} r={PORT_R + 4} className="fill-primary/0 hover:fill-primary/20 transition-all" />
              <circle
                cx={x}
                cy={y}
                r={PORT_R}
                className="fill-primary stroke-content1 group-hover/port:scale-125 transition-transform shadow-sm"
                strokeWidth={2}
              />
            </g>
          );
        })}
      </g>
    );
  };

  /** 渲染一条边 */
  const renderEdge = (edge: RouteEdge) => {
    const src = topology.nodes.find((n) => n.id === edge.source);
    const dst = topology.nodes.find((n) => n.id === edge.target);
    if (!src || !dst) return null;
    const srcPort = edge.sourcePort ?? ROUTE_NODE_CATALOG_MAP[src.type].defaultPorts.find((p) => p.type === 'out')?.id ?? 'out';
    const srcPos = { x: src.position.x + portOffset(src, srcPort, 'out').x, y: src.position.y + portOffset(src, srcPort, 'out').y };
    const dstPos = { x: dst.position.x + portOffset(dst, 'in', 'in').x, y: dst.position.y + portOffset(dst, 'in', 'in').y };
    const midX = (srcPos.x + dstPos.x) / 2;
    const midY = (srcPos.y + dstPos.y) / 2;
    return (
      <g key={edge.id} className="group">
        {/* 底层粗线（hover 高亮） */}
        <path d={bezier(srcPos, dstPos)} fill="none" className="stroke-primary/0 group-hover:stroke-primary/20" strokeWidth={8} />
        {/* 主线 */}
        <path d={bezier(srcPos, dstPos)} fill="none" className="stroke-primary/60 group-hover:stroke-primary" strokeWidth={2.5} markerEnd="url(#arrow-route)" />
        {/* 流动动画线 */}
        <path d={bezier(srcPos, dstPos)} fill="none" className="stroke-primary edge-flow" strokeWidth={1.5} opacity={0.5} />
        {/* 边标签 */}
        {edge.label && (
          <foreignObject x={midX - 24} y={midY - 9} width={48} height={18}>
            <div className="flex justify-center">
              <Chip size="sm" variant="flat" color="primary" className="text-[9px] h-4 min-w-0 px-1">{edge.label}</Chip>
            </div>
          </foreignObject>
        )}
        {/* 删除按钮（hover 显示） */}
        <g transform={`translate(${midX - 9}, ${midY - 9})`} className="opacity-0 group-hover:opacity-100 cursor-pointer" onClick={() => deleteEdge(edge.id)}>
          <circle cx={9} cy={9} r={8} className="fill-danger" />
          <foreignObject x={3} y={3} width={12} height={12}>
            <Trash2 className="w-3 h-3 text-white" />
          </foreignObject>
        </g>
      </g>
    );
  };

  const renderTempEdge = () => {
    if (drag?.type !== 'edge' || !drag.fromPort) return null;
    const src = topology.nodes.find((n) => n.id === drag.fromPort!.nodeId);
    if (!src) return null;
    const srcPos = { x: src.position.x + portOffset(src, drag.fromPort.portId, 'out').x, y: src.position.y + portOffset(src, drag.fromPort.portId, 'out').y };
    return <path d={bezier(srcPos, drag.cursor)} fill="none" className="stroke-primary/60" strokeWidth={2.5} strokeDasharray="5 3" />;
  };

  return (
    <div className="relative w-full h-full rounded-xl overflow-hidden border border-default-200 bg-content2/30 select-none">
      <svg
        ref={svgRef}
        className="w-full h-full cursor-grab active:cursor-grabbing"
        role="application"
        aria-label="路由拓扑画板"
        onWheel={handleWheel}
        onMouseDown={startCanvasPan}
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={() => setDrag(null)}
        onClick={() => fireSelect(null)}
      >
        <defs>
          <marker id="arrow-route" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth">
            <path d="M0,0 L0,6 L8,3 z" className="fill-primary" />
          </marker>
          {/* 点阵网格 */}
          <pattern id="dot-grid-route" width="24" height="24" patternUnits="userSpaceOnUse">
            <circle cx="2" cy="2" r="1" className="fill-default-200" />
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#dot-grid-route)" />

        {/* 包含 Pan & Zoom 平移缩放的 SVG 变形组 */}
        <g transform={`translate(${pan.x}, ${pan.y}) scale(${zoom})`}>
          {topology.edges.map(renderEdge)}
          {renderTempEdge()}
          {topology.nodes.map(renderNode)}
        </g>
      </svg>

      {/* 右上角浮动工具栏：一键排版、缩放、重置 */}
      <div className="absolute top-3 right-3 flex items-center gap-1.5 p-1 bg-content1/80 backdrop-blur-md rounded-lg border border-default-200 shadow-md">
        <Tooltip content="一键自动整理排版拓扑图">
          <Button
            size="sm"
            variant="flat"
            color="primary"
            className="h-7 px-2.5 text-[11px] font-medium gap-1"
            onPress={handleAutoLayout}
          >
            <LayoutGrid className="w-3.5 h-3.5 text-primary" />
            一键自动排版
          </Button>
        </Tooltip>

        <div className="w-[1px] h-4 bg-default-200 mx-0.5" />

        <Tooltip content="缩小 (Scroll Out)">
          <Button
            isIconOnly
            size="sm"
            variant="light"
            className="w-7 h-7 min-w-0"
            onPress={() => setZoom((z) => Math.max(0.4, Number((z - 0.1).toFixed(2))))}
          >
            <ZoomOut className="w-3.5 h-3.5" />
          </Button>
        </Tooltip>

        <span className="text-[10px] font-mono font-bold text-default-600 px-1 min-w-[36px] text-center">
          {Math.round(zoom * 100)}%
        </span>

        <Tooltip content="放大 (Scroll In)">
          <Button
            isIconOnly
            size="sm"
            variant="light"
            className="w-7 h-7 min-w-0"
            onPress={() => setZoom((z) => Math.min(2.0, Number((z + 0.1).toFixed(2))))}
          >
            <ZoomIn className="w-3.5 h-3.5" />
          </Button>
        </Tooltip>

        <Tooltip content="重置画布视角 (100%)">
          <Button
            isIconOnly
            size="sm"
            variant="light"
            className="w-7 h-7 min-w-0"
            onPress={() => { setPan({ x: 0, y: 0 }); setZoom(1.0); }}
          >
            <Maximize2 className="w-3.5 h-3.5" />
          </Button>
        </Tooltip>

        <Chip size="sm" variant="flat" color="default" className="text-[10px] h-6 px-1.5 ml-1">
          {topology.nodes.length} 节点
        </Chip>
      </div>

      {/* 空画布提示 */}
      {topology.nodes.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div className="text-center">
            <Plus className="w-10 h-10 text-default-300 mx-auto mb-2" />
            <p className="text-sm text-default-400">从左侧拖入节点到此处开始编排路由拓扑</p>
          </div>
        </div>
      )}
    </div>
  );
}

/** 路由节点工具箱（左侧 palette） */
export function RouteNodePalette() {
  const categories: Array<{ key: 'source' | 'filter' | 'action'; label: string }> = [
    { key: 'source', label: '呼入源' },
    { key: 'filter', label: '过滤条件' },
    { key: 'action', label: '执行动作' },
  ];
  return (
    <div className="w-64 shrink-0 h-full flex flex-col bg-content1 rounded-xl border border-default-200 overflow-hidden">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-default-200 shrink-0 bg-content2/50">
        <Plus className="w-4 h-4 text-primary" />
        <span className="text-xs font-bold text-foreground">路由节点</span>
      </div>
      <div className="flex-1 overflow-y-auto p-3 flex flex-col gap-3">
        <p className="text-[10px] text-default-400 px-1">拖拽卡片到画布创建节点</p>
        {categories.map((cat) => {
          const items = ROUTE_NODE_CATALOG.filter((n) => n.category === cat.key);
          if (items.length === 0) return null;
          return (
            <div key={cat.key} className="flex flex-col gap-1.5">
              <span className="text-[10px] font-bold text-default-500 uppercase tracking-wider px-1">{cat.label}</span>
              {items.map((entry) => {
                const Icon = ICON_MAP[entry.icon] ?? Plus;
                return (
                  <div
                    key={entry.type}
                    draggable
                    onDragStart={(e) => { e.dataTransfer.setData('application/route-node', JSON.stringify(entry)); e.dataTransfer.effectAllowed = 'copy'; }}
                    className={`p-2.5 rounded-lg border ${entry.color} cursor-grab active:cursor-grabbing hover:shadow-sm hover:scale-[1.02] transition-all flex items-center gap-2.5 bg-content1`}
                  >
                    <div className="w-7 h-7 rounded-lg flex items-center justify-center bg-content2 shrink-0">
                      <Icon className="w-3.5 h-3.5" />
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
    </div>
  );
}

/** 路由节点属性面板（右侧 inspector） */
interface RouteInspectorProps {
  node: RouteNode | null;
  onChange: (node: RouteNode) => void;
}

export function RouteNodeInspector({ node, onChange }: RouteInspectorProps) {
  if (!node) {
    return (
      <div className="w-72 shrink-0 h-full p-4 bg-content1 rounded-xl border border-default-200 flex items-center justify-center">
        <div className="text-center">
          <Sparkles className="w-8 h-8 text-default-300 mx-auto mb-2" />
          <p className="text-xs text-default-400">在画布中点击选中节点查看属性</p>
        </div>
      </div>
    );
  }
  const cat = ROUTE_NODE_CATALOG_MAP[node.type];
  const Icon = ICON_MAP[cat.icon] ?? Plus;
  return (
    <div className="w-72 shrink-0 h-full flex flex-col bg-content1 rounded-xl border border-default-200 overflow-hidden">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-default-200 shrink-0 bg-content2/50">
        <Sparkles className="w-4 h-4 text-primary" />
        <span className="text-xs font-bold text-foreground">节点属性</span>
      </div>
      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-3">
        <div className={`p-3 rounded-lg border ${cat.color} flex items-center gap-2.5`}>
          <div className="w-8 h-8 rounded-lg flex items-center justify-center bg-content2">
            <Icon className="w-4 h-4" />
          </div>
          <div className="flex flex-col min-w-0">
            <span className="text-xs font-bold truncate">{node.title}</span>
            <span className="text-[10px] text-default-400 font-mono">{node.id}</span>
          </div>
        </div>
        <div className="flex flex-col gap-1.5">
          <label className="text-[11px] font-semibold text-default-500">节点标题</label>
          <input
            className="text-xs px-3 py-2 rounded-lg border border-default-200 bg-content2 text-foreground focus:outline-none focus:border-primary"
            value={node.title}
            onChange={(e) => onChange({ ...node, title: e.target.value })}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <label className="text-[11px] font-semibold text-default-500">节点描述</label>
          <textarea
            className="text-xs px-3 py-2 rounded-lg border border-default-200 bg-content2 text-foreground min-h-14 focus:outline-none focus:border-primary"
            value={node.description ?? ''}
            onChange={(e) => onChange({ ...node, description: e.target.value })}
          />
        </div>
        <div className="flex flex-col gap-2 pt-2 border-t border-default-200">
          <span className="text-[11px] font-semibold text-default-500">配置参数</span>
          <pre className="text-[10px] p-2 rounded-lg bg-content2 text-default-600 overflow-auto max-h-60 font-mono">
            {JSON.stringify(node.config, null, 2)}
          </pre>
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
