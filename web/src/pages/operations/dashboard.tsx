// 运营监控 - 仪表盘
// 精炼工业级监控台风格：Hero Banner + KPI 双层 + 主图表 + QoS + 系统监控
// 所有颜色使用 HeroUI 语义令牌（text-primary / bg-content2 等），适配暗黑模式。

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Chip, Progress, Spinner,
} from '@heroui/react';
import {
  RefreshCw, Activity, Sparkles, Server,
  PhoneCall, Users, BarChart2, Radio, Award, AudioLines,
  Clock, Gauge, Zap, CheckCircle2, Shield, Cpu, Database, HardDrive,
} from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { valueText } from '@/pages/shared/format';

/** 测量容器实际渲染宽度，让 SVG viewBox 与渲染区一致，避免鼠标映射偏移。
 * 使用 callback ref 模式：即使元素在 early return 之后才挂载，也能正确测量。 */
function useElementWidth<T extends HTMLElement>(fallback: number) {
  const [width, setWidth] = useState(fallback);
  const observerRef = useRef<ResizeObserver | null>(null);

  const ref = useCallback((node: T | null) => {
    if (observerRef.current) {
      observerRef.current.disconnect();
      observerRef.current = null;
    }
    if (!node) return;
    const measure = () => setWidth(node.getBoundingClientRect().width);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    observerRef.current = observer;
  }, []);

  return { ref, width };
}

/** KPI 卡片迷你趋势线：数据少于 2 个点时不渲染 */
function Sparkline({ data, color = 'stroke-primary' }: { data: number[]; color?: string }) {
  if (data.length < 2) return null;
  const max = Math.max(...data, 1);
  const min = Math.min(...data, 0);
  const range = max - min || 1;
  const w = 100, h = 28;
  const points = data.map((v, i) => `${(i / (data.length - 1)) * w},${h - ((v - min) / range) * (h - 4) - 2}`).join(' ');
  const areaPoints = `0,${h} ${points} ${w},${h}`;
  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} className="overflow-visible" aria-hidden="true" preserveAspectRatio="none">
      <defs>
        <linearGradient id={`spark-${color}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" className={color} stopColor="currentColor" stopOpacity="0.3" />
          <stop offset="100%" className={color} stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      <polygon points={areaPoints} fill={`url(#spark-${color})`} className={color} />
      <polyline points={points} fill="none" className={`${color} transition-all duration-500 ease-out`} strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export interface HourlyTrendItem {
  hour: string;
  total_calls: number;
  answered_calls: number;
}

export interface Summary {
  active_calls?: number;
  today_total_calls?: number;
  today_answered_calls?: number;
  today_canceled_calls?: number;
  today_failed_calls?: number;
  answer_rate?: number;
  registered_users?: number;
  active_gateways?: number;
  // 字段名与后端 DashboardStats 对齐：avg_mos / avg_loss_rate / avg_jitter_ms
  avg_mos?: number;
  avg_loss_rate?: number;
  avg_jitter_ms?: number;
  avg_duration_secs?: number;
  ner_rate?: number;
  hourly_trends?: HourlyTrendItem[];
}

/** 24h 呼叫趋势图：渐变面积 + 网格刻度 + hover crosshair tooltip */
function HourlyTrendsSection({ trends }: { trends?: HourlyTrendItem[] }) {
  const hasData = Boolean(trends && trends.length > 0 && trends.some((t) => t.total_calls > 0));
  const defaultHours = Array.from({ length: 24 }, (_, i) => `${String(i).padStart(2, '0')}:00`);
  const displayData = hasData && trends
    ? trends
    : defaultHours.map((hour) => ({ hour, total_calls: 0, answered_calls: 0 }));

  const maxVal = Math.max(...displayData.map((d) => d.total_calls), 10);
  const chartHeight = 180;
  const { ref: chartRef, width: chartWidth } = useElementWidth<HTMLDivElement>(700);
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);

  const pointsTotal = displayData.map((d, idx) => {
    const x = (idx / (displayData.length - 1)) * chartWidth;
    const y = chartHeight - (d.total_calls / maxVal) * (chartHeight - 24) - 4;
    return `${x},${y}`;
  }).join(' ');

  const pointsAnswered = displayData.map((d, idx) => {
    const x = (idx / (displayData.length - 1)) * chartWidth;
    const y = chartHeight - (d.answered_calls / maxVal) * (chartHeight - 24) - 4;
    return `${x},${y}`;
  }).join(' ');

  const areaAnswered = `0,${chartHeight} ${pointsAnswered} ${chartWidth},${chartHeight}`;
  const areaTotal = `0,${chartHeight} ${pointsTotal} ${chartWidth},${chartHeight}`;

  const totalSum = displayData.reduce((s, d) => s + d.total_calls, 0);
  const answeredSum = displayData.reduce((s, d) => s + d.answered_calls, 0);
  const overallAsr = totalSum > 0 ? (answeredSum / totalSum) * 100 : 0;

  const handleMouseMove = (e: React.MouseEvent<SVGSVGElement>) => {
    if (!hasData) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const pct = x / rect.width;
    const idx = Math.min(Math.max(Math.round(pct * (displayData.length - 1)), 0), displayData.length - 1);
    setHoveredIdx(idx);
  };

  const handleMouseLeave = () => setHoveredIdx(null);
  const hoveredData = hoveredIdx !== null ? displayData[hoveredIdx] : null;
  const hoveredX = hoveredIdx !== null ? (hoveredIdx / (displayData.length - 1)) * chartWidth : 0;

  return (
    <Card shadow="sm" className="w-full dash-enter" style={{ animationDelay: '120ms' }}>
      <CardBody className="p-5 flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <BarChart2 className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">24 小时话务趋势</h3>
            <span className="text-tiny text-default-400 font-mono">·</span>
            <span className="text-tiny text-default-500">每小时呼叫量与接通率分布</span>
          </div>
          <div className="flex items-center gap-3 text-tiny">
            <span className="flex items-center gap-1.5">
              <span className="w-2.5 h-2.5 rounded-sm bg-primary" />
              <span className="text-default-500">总量</span>
            </span>
            <span className="flex items-center gap-1.5">
              <span className="w-2.5 h-2.5 rounded-sm bg-success" />
              <span className="text-default-500">应答</span>
            </span>
            <span className="text-default-300">|</span>
            <span className="flex items-center gap-1.5">
              <span className="text-default-500">ASR</span>
              <span className="font-bold text-primary font-mono tnum">{overallAsr.toFixed(1)}%</span>
            </span>
          </div>
        </div>

        <div className="relative" ref={chartRef}>
          {!hasData && (
            <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none z-10">
              <Activity className="w-8 h-8 text-default-300 mb-1.5" />
              <p className="text-tiny text-default-400 font-medium">今日暂无通话记录</p>
              <p className="text-[10px] text-default-300">发起呼叫后将实时绘制话务趋势</p>
            </div>
          )}
          <svg
            viewBox={`0 0 ${chartWidth} ${chartHeight}`}
            className="w-full overflow-visible cursor-crosshair"
            style={{ height: `${chartHeight}px` }}
            onMouseMove={handleMouseMove}
            onMouseLeave={handleMouseLeave}
            role="img"
            aria-label="24 小时呼叫趋势图"
          >
            <defs>
              <linearGradient id="totalGradient" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" className="text-primary" stopColor="currentColor" stopOpacity="0.28" />
                <stop offset="100%" className="text-primary" stopColor="currentColor" stopOpacity="0" />
              </linearGradient>
              <linearGradient id="answeredGradient" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" className="text-success" stopColor="currentColor" stopOpacity="0.35" />
                <stop offset="100%" className="text-success" stopColor="currentColor" stopOpacity="0.02" />
              </linearGradient>
            </defs>

            {/* 水平网格线 + Y 轴刻度 */}
            {[0, 0.25, 0.5, 0.75, 1].map((p) => {
              const y = chartHeight - p * (chartHeight - 24) - 4;
              return (
                <g key={p}>
                  <line x1="0" y1={y} x2={chartWidth} y2={y} className="stroke-default-100" strokeWidth="1" strokeDasharray={p === 0 ? '0' : '3 4'} />
                  <text x={6} y={y - 4} className="fill-default-400 text-[9px] font-mono tnum">
                    {Math.round(maxVal * p)}
                  </text>
                </g>
              );
            })}

            <polygon points={areaTotal} fill="url(#totalGradient)" />
            <polygon points={areaAnswered} fill="url(#answeredGradient)" />

            <polyline fill="none" className="stroke-primary transition-all duration-500 ease-out" strokeWidth="2.5" points={pointsTotal} strokeLinecap="round" strokeLinejoin="round" />
            <polyline fill="none" className="stroke-success transition-all duration-500 ease-out" strokeWidth="2" points={pointsAnswered} strokeLinecap="round" strokeLinejoin="round" />

            {displayData.map((d, idx) => {
              const x = (idx / (displayData.length - 1)) * chartWidth;
              const y = chartHeight - (d.total_calls / maxVal) * (chartHeight - 24) - 4;
              if (d.total_calls === 0) {
                return <circle key={d.hour} cx={x} cy={y} r="1.5" className="fill-default-300" opacity="0.4" />;
              }
              return <circle key={d.hour} cx={x} cy={y} r="3" className="fill-primary stroke-background" strokeWidth="1.5" />;
            })}

            {hoveredIdx !== null && (
              <line x1={hoveredX} y1={0} x2={hoveredX} y2={chartHeight} className="stroke-default-400" strokeWidth="1.5" strokeDasharray="3 3" />
            )}
          </svg>

          {hoveredIdx !== null && hoveredData && (
            <div
              className="absolute z-50 pointer-events-none bg-background/95 backdrop-blur shadow-lg border border-default-200 p-2.5 rounded-lg text-tiny font-mono flex flex-col gap-1 w-44 dash-enter"
              style={{ left: `${Math.min(Math.max((hoveredX / chartWidth) * 100, 5), 75)}%`, top: '8px' }}
            >
              <div className="font-bold border-b border-default-200 pb-1 mb-1 text-foreground text-center tnum">
                {hoveredData.hour}
              </div>
              <div className="flex justify-between gap-4">
                <span className="text-default-500 flex items-center gap-1.5">
                  <span className="w-2 h-2 rounded-full bg-primary" /> 总量
                </span>
                <span className="font-bold text-primary tnum">{hoveredData.total_calls}</span>
              </div>
              <div className="flex justify-between gap-4">
                <span className="text-default-500 flex items-center gap-1.5">
                  <span className="w-2 h-2 rounded-full bg-success" /> 应答
                </span>
                <span className="font-bold text-success tnum">{hoveredData.answered_calls}</span>
              </div>
              <div className="flex justify-between gap-4 pt-1 border-t border-default-200">
                <span className="text-default-500">ASR</span>
                <span className="font-bold text-primary tnum">
                  {hoveredData.total_calls > 0
                    ? `${((hoveredData.answered_calls / hoveredData.total_calls) * 100).toFixed(2)}%`
                    : '0.00%'}
                </span>
              </div>
            </div>
          )}
        </div>

        <div className="flex justify-between text-[10px] text-default-400 px-1 font-mono tnum">
          {displayData.map((d, idx) => {
            const showLabel = idx % 3 === 0 || idx === displayData.length - 1;
            return (
              <span key={idx} className={showLabel ? '' : 'opacity-0'}>{d.hour}</span>
            );
          })}
        </div>
      </CardBody>
    </Card>
  );
}

/** MOS 半圆仪表盘：score 1.0-4.5 映射到 0-180 度，带 3.5/4.0 阈值刻度 */
function MosGauge({ score }: { score: number }) {
  const clampedScore = Math.max(1, Math.min(4.5, score));
  const angle = ((clampedScore - 1) / 3.5) * 180;
  const colorClass = clampedScore >= 4.0 ? 'text-success' : clampedScore >= 3.5 ? 'text-warning' : 'text-danger';
  const r = 50;
  const arcLength = Math.PI * r;
  const thresholdAngle = (threshold: number) => ((threshold - 1) / 3.5) * 180;
  const angleToPoint = (deg: number, radius: number) => {
    const rad = (deg * Math.PI) / 180;
    return { x: 60 - radius * Math.cos(rad), y: 60 - radius * Math.sin(rad) };
  };
  const tick3_5 = angleToPoint(thresholdAngle(3.5), r);
  const tick4_0 = angleToPoint(thresholdAngle(4.0), r);
  const tick3_5Outer = angleToPoint(thresholdAngle(3.5), r + 6);
  const tick4_0Outer = angleToPoint(thresholdAngle(4.0), r + 6);
  return (
    <svg width={120} height={76} viewBox="0 0 120 76" className={`shrink-0 ${colorClass}`} role="img" aria-label={`MOS 语音质量评分 ${clampedScore.toFixed(2)}`}>
      <path d="M 10 60 A 50 50 0 0 1 110 60" fill="none" stroke="currentColor" strokeWidth={8} className="text-default-200" />
      <path
        d="M 10 60 A 50 50 0 0 1 110 60"
        fill="none"
        stroke="currentColor"
        strokeWidth={8}
        strokeDasharray={`${(angle / 180) * arcLength} ${arcLength}`}
        strokeLinecap="round"
      />
      <line x1={tick3_5.x} y1={tick3_5.y} x2={tick3_5Outer.x} y2={tick3_5Outer.y} className="stroke-warning" strokeWidth={1.5} />
      <line x1={tick4_0.x} y1={tick4_0.y} x2={tick4_0Outer.x} y2={tick4_0Outer.y} className="stroke-success" strokeWidth={1.5} />
      <text x={tick3_5.x - 2} y={tick3_5.y + 12} className="fill-warning text-[8px] font-mono">3.5</text>
      <text x={tick4_0.x - 2} y={tick4_0.y + 12} className="fill-success text-[8px] font-mono">4.0</text>
      <text x={60} y={50} textAnchor="middle" className="fill-foreground text-lg font-bold font-mono tnum">{clampedScore.toFixed(2)}</text>
    </svg>
  );
}

/** MOS 语音质量与媒体 QoS 监控（无采样时展示 0 值基线，不空白） */
function MosQualitySection({ data }: { data: Summary }) {
  const mosScore = data.avg_mos ?? 0;
  const lossRate = data.avg_loss_rate ?? 0;
  const jitterMs = data.avg_jitter_ms ?? 0;

  let mosColor: 'success' | 'warning' | 'danger' | 'default' = 'default';
  let mosLabel = '等待 RTP 媒体流';
  if (mosScore >= 4.0) { mosColor = 'success'; mosLabel = '极佳 (Excellent)'; }
  else if (mosScore >= 3.5) { mosColor = 'warning'; mosLabel = '良好 (Good)'; }
  else if (mosScore > 0) { mosColor = 'danger'; mosLabel = '较差 (Poor)'; }

  return (
    <Card shadow="sm" className="dash-enter h-full" style={{ animationDelay: '320ms' }}>
      <CardBody className="p-5 flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <AudioLines className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">媒体 QoS</h3>
          </div>
          <Chip size="sm" variant="flat" color={mosColor}>{mosLabel}</Chip>
        </div>

        <div className="p-4 bg-content2 rounded-xl flex items-center justify-between gap-3">
          <div className="flex flex-col gap-1">
            <div className="text-tiny text-default-500 font-medium">平均 MOS 分数</div>
            <div className={`text-3xl font-bold font-mono tnum ${mosScore >= 4 ? 'text-success' : mosScore >= 3.5 ? 'text-warning' : mosScore > 0 ? 'text-danger' : 'text-foreground'}`}>
              {mosScore.toFixed(2)}
            </div>
            <div className="text-[10px] text-default-400 font-mono">/ 5.00 · PESQ</div>
          </div>
          <MosGauge score={mosScore} />
        </div>

        <div className="grid grid-cols-2 gap-2">
          <div className="p-3 rounded-lg bg-content2/60 flex flex-col gap-1">
            <span className="text-[10px] text-default-400 font-medium">平均丢包率</span>
            <span className="text-base font-bold text-foreground font-mono tnum">{lossRate.toFixed(2)}%</span>
          </div>
          <div className="p-3 rounded-lg bg-content2/60 flex flex-col gap-1">
            <span className="text-[10px] text-default-400 font-medium">平均抖动</span>
            <span className="text-base font-bold text-foreground font-mono tnum">{jitterMs.toFixed(2)} ms</span>
          </div>
        </div>
      </CardBody>
    </Card>
  );
}

/** 呼叫成功率与 ASR/NER 分析（无数据时展示 0 值基线，不空白） */
function SuccessRateSection({ data }: { data: Summary }) {
  const hasCalls = Boolean(data.today_total_calls && data.today_total_calls > 0);
  const asr = (data.answer_rate ?? 0) * 100;
  const ner = (data.ner_rate ?? 0) * 100;

  return (
    <Card shadow="sm" className="dash-enter h-full" style={{ animationDelay: '360ms' }}>
      <CardBody className="p-5 flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Gauge className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">信令质量</h3>
          </div>
          <Chip size="sm" variant="flat" color={hasCalls ? 'success' : 'default'}>
            {hasCalls ? '正常统计' : '无呼叫记录'}
          </Chip>
        </div>

        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5 p-3 rounded-medium bg-content2">
            <div className="flex justify-between text-tiny">
              <span className="font-medium text-default-500">接通率 (ASR)</span>
              <span className="font-bold font-mono text-success tnum">{asr.toFixed(3)}%</span>
            </div>
            <Progress
              size="sm"
              value={asr}
              color={asr >= 80 ? 'success' : asr >= 50 ? 'warning' : 'danger'}
              aria-label="接通率 ASR"
            />
          </div>

          <div className="flex flex-col gap-1.5 p-3 rounded-medium bg-content2">
            <div className="flex justify-between text-tiny">
              <span className="font-medium text-default-500">网络有效到达率 (NER)</span>
              <span className="font-bold font-mono text-primary tnum">{ner.toFixed(3)}%</span>
            </div>
            <Progress size="sm" value={ner} color="primary" aria-label="网络有效到达率 NER" />
          </div>

          <div className="grid grid-cols-3 gap-2 text-center">
            <div className="p-2 rounded-medium bg-content2/60">
              <div className="text-[10px] text-default-400">已接通</div>
              <div className="text-sm font-bold text-success font-mono tnum">{data.today_answered_calls ?? 0}</div>
            </div>
            <div className="p-2 rounded-medium bg-content2/60">
              <div className="text-[10px] text-default-400">已取消</div>
              <div className="text-sm font-bold text-warning font-mono tnum">{data.today_canceled_calls ?? 0}</div>
            </div>
            <div className="p-2 rounded-medium bg-content2/60">
              <div className="text-[10px] text-default-400">失败</div>
              <div className="text-sm font-bold text-danger font-mono tnum">{data.today_failed_calls ?? 0}</div>
            </div>
          </div>
        </div>
      </CardBody>
    </Card>
  );
}

export interface NodeTrafficItem {
  hour: string;
  kbps: number;
}

export interface NodeTrafficData {
  node_id: string;
  node_type: string;
  series: NodeTrafficItem[];
}

function NodeTrafficSection() {
  const [trafficData, setTrafficData] = useState<NodeTrafficData[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedType, setSelectedType] = useState<'all' | 'sip' | 'media'>('all');
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);
  const [hiddenNodes, setHiddenNodes] = useState<Set<string>>(new Set());
  const chartHeight = 180;
  const { ref: chartRef, width: chartWidth } = useElementWidth<HTMLDivElement>(800);

  const loadTraffic = useCallback(async () => {
    try {
      const res = await api.get<NodeTrafficData[]>('/overview/node-traffic');
      setTrafficData(res);
    } catch (e) {
      console.error('加载节点流量失败:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadTraffic();
    const timer = setInterval(() => { void loadTraffic(); }, 10000);
    return () => clearInterval(timer);
  }, [loadTraffic]);

  if (loading) {
    return (
      <Card shadow="sm" className="w-full dash-enter" style={{ animationDelay: '200ms' }}>
        <CardBody className="p-8 flex justify-center items-center">
          <Spinner label="加载节点流量数据中..." />
        </CardBody>
      </Card>
    );
  }

  const filteredData = trafficData.filter(d => selectedType === 'all' || d.node_type === selectedType);
  const allHours = trafficData[0]?.series.map(s => s.hour) || [];
  const maxVal = Math.max(...filteredData.flatMap(d => d.series.map(s => s.kbps)), 100);

  const NODE_COLORS: Array<{ stroke: string; fill: string; dot: string; text: string }> = [
    { stroke: 'stroke-primary', fill: 'fill-primary', dot: 'bg-primary', text: 'text-primary' },
    { stroke: 'stroke-success', fill: 'fill-success', dot: 'bg-success', text: 'text-success' },
    { stroke: 'stroke-warning', fill: 'fill-warning', dot: 'bg-warning', text: 'text-warning' },
  ];
  const NODE_COLOR_MAP: Record<string, number> = {
    'sip-edge-01': 0, 'sip-edge-standalone': 0,
    'media-node-01': 1, 'local-media': 1,
  };
  const getNodeColor = (id: string, index: number) => {
    const mappedIdx = NODE_COLOR_MAP[id];
    return NODE_COLORS[(mappedIdx !== undefined ? mappedIdx : index) % NODE_COLORS.length];
  };

  const formatKbps = (kbps: number) => kbps >= 1000 ? `${(kbps / 1000).toFixed(1)} Mbps` : `${kbps} Kbps`;

  const handleMouseMove = (e: React.MouseEvent<SVGSVGElement>) => {
    if (allHours.length === 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const pct = x / rect.width;
    const idx = Math.min(Math.max(Math.round(pct * (allHours.length - 1)), 0), allHours.length - 1);
    setHoveredIdx(idx);
  };
  const handleMouseLeave = () => setHoveredIdx(null);
  const hoveredX = hoveredIdx !== null && allHours.length > 1 ? (hoveredIdx / (allHours.length - 1)) * chartWidth : 0;
  const hoveredTime = hoveredIdx !== null ? allHours[hoveredIdx] : '';

  return (
    <Card shadow="sm" className="w-full dash-enter" style={{ animationDelay: '240ms' }}>
      <CardBody className="p-5 flex flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Radio className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">节点流量</h3>
            <span className="text-tiny text-default-400 font-mono">·</span>
            <span className="text-tiny text-default-500">近 24 小时每秒带宽</span>
          </div>
          <div className="flex items-center gap-1 p-0.5 rounded-medium bg-content2">
            {(['all', 'sip', 'media'] as const).map((t) => (
              <Button
                key={t}
                size="sm"
                variant={selectedType === t ? 'solid' : 'light'}
                color={selectedType === t ? 'primary' : 'default'}
                onPress={() => setSelectedType(t)}
                className="h-7 min-w-14 px-2 data-[hover=true]:bg-default-100"
              >
                {t === 'all' ? '全部' : t === 'sip' ? '信令' : '媒体'}
              </Button>
            ))}
          </div>
        </div>

        {filteredData.length === 0 ? (
          <div className="py-8 flex flex-col items-center justify-center bg-content2/30 rounded-xl border border-dashed border-default-200">
            <Activity className="w-8 h-8 text-default-400 opacity-60 mb-2" />
            <p className="text-tiny text-default-400">没有匹配的节点流量数据</p>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="flex flex-wrap gap-2">
              {filteredData.map((node, idx) => {
                const c = getNodeColor(node.node_id, idx);
                const latestKbps = node.series[node.series.length - 1]?.kbps || 0;
                const isHidden = hiddenNodes.has(node.node_id);
                return (
                  <div
                    key={node.node_id}
                    onClick={() => setHiddenNodes(prev => {
                      const next = new Set(prev);
                      if (next.has(node.node_id)) next.delete(node.node_id);
                      else next.add(node.node_id);
                      return next;
                    })}
                    className={`flex items-center gap-1.5 bg-content2/60 px-2.5 py-1 rounded-medium cursor-pointer select-none transition-all ${isHidden ? 'opacity-30' : 'hover:bg-content2'}`}
                  >
                    <span className={`w-2 h-2 rounded-full ${c.dot}`} />
                    <span className="font-mono font-semibold text-foreground text-tiny">{node.node_id}</span>
                    <span className="text-default-400 text-[10px] font-mono">{node.node_type === 'sip' ? 'SIP' : 'RTP'}</span>
                    <span className={`font-mono font-bold text-tiny tnum ${c.text}`}>{formatKbps(latestKbps)}/s</span>
                  </div>
                );
              })}
            </div>

            <div className="relative" ref={chartRef}>
              <svg
                viewBox={`0 0 ${chartWidth} ${chartHeight}`}
                className="w-full overflow-visible cursor-crosshair"
                style={{ height: `${chartHeight}px` }}
                onMouseMove={handleMouseMove}
                onMouseLeave={handleMouseLeave}
                role="img"
                aria-label="节点流量趋势图"
              >
                {/* 水平网格线 + Y 轴刻度 */}
                {[0, 0.25, 0.5, 0.75, 1].map((p) => {
                  const y = chartHeight - p * (chartHeight - 30) - 4;
                  return (
                    <g key={p}>
                      <line x1="0" y1={y} x2={chartWidth} y2={y} className="stroke-default-100" strokeWidth="1" strokeDasharray={p === 0 ? '0' : '3 4'} />
                      <text x="6" y={y - 4} className="fill-default-400 text-[9px] font-mono tnum">
                        {formatKbps(Math.round(maxVal * p))}
                      </text>
                    </g>
                  );
                })}

                {filteredData.filter(d => !hiddenNodes.has(d.node_id)).map((node, nodeIdx) => {
                  const c = getNodeColor(node.node_id, nodeIdx);
                  const points = node.series.map((item, idx) => {
                    const x = (idx / (node.series.length - 1)) * chartWidth;
                    const y = chartHeight - (item.kbps / maxVal) * (chartHeight - 30) - 4;
                    return `${x},${y}`;
                  }).join(' ');
                  return (
                    <g key={node.node_id}>
                      <polyline
                        fill="none"
                        strokeWidth="2.5"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        points={points}
                        className={`${c.stroke} transition-all duration-500 ease-out drop-shadow-sm`}
                      />
                      {node.series.map((item, idx) => {
                        if (idx % 3 !== 0 && idx !== node.series.length - 1) return null;
                        const x = (idx / (node.series.length - 1)) * chartWidth;
                        const y = chartHeight - (item.kbps / maxVal) * (chartHeight - 30) - 4;
                        return <circle key={idx} cx={x} cy={y} r="3" className={`${c.fill} stroke-background`} strokeWidth="1.5" />;
                      })}
                    </g>
                  );
                })}

                {hoveredIdx !== null && (
                  <line x1={hoveredX} y1={0} x2={hoveredX} y2={chartHeight} className="stroke-default-400" strokeWidth="1.5" strokeDasharray="3 3" />
                )}
              </svg>

              {hoveredIdx !== null && hoveredTime && (
                <div
                  className="absolute z-50 pointer-events-none bg-background/95 backdrop-blur shadow-lg border border-default-200 p-2.5 rounded-lg text-tiny font-mono flex flex-col gap-1 w-52 dash-enter"
                  style={{ left: `${Math.min(Math.max((hoveredX / chartWidth) * 100, 5), 72)}%`, top: '8px' }}
                >
                  <div className="font-bold border-b border-default-200 pb-1 mb-1 text-foreground text-center tnum">
                    {hoveredTime}
                  </div>
                  {filteredData.filter(d => !hiddenNodes.has(d.node_id)).map((node, nodeIdx) => {
                    const c = getNodeColor(node.node_id, nodeIdx);
                    const val = node.series[hoveredIdx]?.kbps || 0;
                    return (
                      <div key={node.node_id} className="flex justify-between gap-4 items-center">
                        <span className="text-default-500 flex items-center gap-1.5 truncate max-w-[120px]">
                          <span className={`w-2 h-2 rounded-full ${c.dot}`} />
                          {node.node_id}
                        </span>
                        <span className={`font-bold tnum ${c.text}`}>{formatKbps(val)}/s</span>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            {allHours.length > 0 && (
              <div className="flex justify-between text-[10px] text-default-400 px-1 font-mono tnum">
                {allHours.map((hour, idx) => {
                  const showLabel = idx % 2 === 0 || idx === allHours.length - 1;
                  return <span key={idx} className={showLabel ? '' : 'opacity-0'}>{hour}</span>;
                })}
              </div>
            )}
          </div>
        )}
      </CardBody>
    </Card>
  );
}

export interface GatewayConcurrency {
  name: string;
  direction: string;
  active_calls: number;
  max_channels: number;
}

export interface SbcSecurityStats {
  blocked_calls_24h: number;
  auth_failures_24h: number;
  error_codes_breakdown: Record<string, number>;
}

export interface SystemResourceStats {
  cpu_percent: number;
  memory_percent: number;
  disk_percent: number;
  db_pool_active: number;
  db_pool_max: number;
}

export interface MonitoringExtras {
  gateways: GatewayConcurrency[];
  security: SbcSecurityStats;
  resources: SystemResourceStats;
}

function MonitoringExtrasSection() {
  const [extras, setExtras] = useState<MonitoringExtras | null>(null);
  const [loading, setLoading] = useState(true);

  const loadExtras = useCallback(async () => {
    try {
      const res = await api.get<MonitoringExtras>('/overview/monitoring-extras');
      setExtras(res);
    } catch (e) {
      console.error('加载扩展监控数据失败:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadExtras();
    const timer = setInterval(() => { void loadExtras(); }, 10000);
    return () => clearInterval(timer);
  }, [loadExtras]);

  if (loading || !extras) {
    return (
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {Array.from({ length: 3 }).map((_, idx) => (
          <Card key={idx} shadow="sm" className="w-full h-48 animate-pulse">
            <CardBody className="flex items-center justify-center">
              <Spinner size="sm" />
            </CardBody>
          </Card>
        ))}
      </div>
    );
  }

  const { gateways, security, resources } = extras;

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
      {/* 中继并发水位 */}
      <Card shadow="sm" className="dash-enter" style={{ animationDelay: '440ms' }}>
        <CardBody className="p-5 flex flex-col gap-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Server className="w-4 h-4 text-primary" />
              <h3 className="text-small font-bold text-foreground">中继并发水位</h3>
            </div>
            <Chip size="sm" variant="flat" color="primary">实时</Chip>
          </div>
          <div className="flex flex-col gap-2 max-h-[280px] overflow-y-auto pr-1">
            {[...gateways].sort((a, b) => b.active_calls - a.active_calls).slice(0, 10).map((gw) => {
              const hasLimit = gw.max_channels > 0;
              const percent = hasLimit ? Math.round((gw.active_calls / gw.max_channels) * 100) : 0;
              return (
                <div key={gw.name} className="p-2.5 bg-content2 rounded-lg flex flex-col gap-1.5">
                  <div className="flex justify-between items-center text-tiny">
                    <div className="flex items-center gap-1.5">
                      <span className={`w-1.5 h-1.5 rounded-full ${gw.direction === 'access' ? 'bg-primary' : 'bg-success'}`} />
                      <span className="font-mono font-bold text-foreground">{gw.name}</span>
                      <span className="text-default-400 text-[9px] font-mono">{gw.direction === 'access' ? '接入' : '落地'}</span>
                    </div>
                    <span className="font-mono font-semibold text-foreground tnum">{gw.active_calls}/{hasLimit ? gw.max_channels : '∞'}</span>
                  </div>
                  {hasLimit ? (
                    <Progress size="sm" value={percent} color={percent >= 85 ? 'danger' : percent >= 60 ? 'warning' : 'success'} aria-label={`${gw.name} 水位`} />
                  ) : (
                    <div className="text-[9px] text-default-400">无最大并发限制</div>
                  )}
                </div>
              );
            })}
          </div>
        </CardBody>
      </Card>

      {/* SBC 安全防御 */}
      <Card shadow="sm" className="dash-enter" style={{ animationDelay: '480ms' }}>
        <CardBody className="p-5 flex flex-col gap-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Shield className="w-4 h-4 text-warning" />
              <h3 className="text-small font-bold text-foreground">SBC 安全防御</h3>
            </div>
            <Chip size="sm" variant="flat" color="warning">24h</Chip>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div className="p-3 rounded-lg bg-danger/10 flex flex-col gap-1">
              <span className="text-[10px] text-danger font-medium">拦截次数</span>
              <span className="text-xl font-bold text-danger font-mono tnum">{security.blocked_calls_24h}</span>
            </div>
            <div className="p-3 rounded-lg bg-warning/10 flex flex-col gap-1">
              <span className="text-[10px] text-warning font-medium">鉴权失败</span>
              <span className="text-xl font-bold text-warning font-mono tnum">{security.auth_failures_24h}</span>
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <span className="text-[10px] text-default-400 font-medium">失败 SIP 响应码分布</span>
            <div className="flex flex-wrap gap-1.5">
              {Object.entries(security.error_codes_breakdown).map(([code, count]) => {
                let color: 'default' | 'primary' | 'warning' | 'danger' = 'default';
                if (code.startsWith('5')) color = 'danger';
                else if (code === '401' || code === '403') color = 'warning';
                else if (code === '404') color = 'primary';
                return (
                  <div key={code} className="flex items-center gap-1 bg-content2 px-2 py-1 rounded-medium text-tiny font-mono">
                    <Chip size="sm" color={color} variant="flat" className="h-4 px-1 text-[10px]">{code}</Chip>
                    <span className="font-bold text-foreground tnum">{count}</span>
                  </div>
                );
              })}
            </div>
          </div>
        </CardBody>
      </Card>

      {/* 系统资源 */}
      <Card shadow="sm" className="dash-enter" style={{ animationDelay: '520ms' }}>
        <CardBody className="p-5 flex flex-col gap-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Cpu className="w-4 h-4 text-success" />
              <h3 className="text-small font-bold text-foreground">宿主机与数据库</h3>
            </div>
            <Chip size="sm" variant="dot" color="success">就绪</Chip>
          </div>

          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-1">
              <div className="flex justify-between text-tiny items-center">
                <span className="text-default-500 font-medium flex items-center gap-1.5"><Cpu className="w-3 h-3" />CPU</span>
                <span className="font-mono font-bold text-foreground tnum">{resources.cpu_percent.toFixed(1)}%</span>
              </div>
              <Progress size="sm" value={resources.cpu_percent} color={resources.cpu_percent >= 80 ? 'danger' : 'success'} aria-label="CPU" />
            </div>
            <div className="flex flex-col gap-1">
              <div className="flex justify-between text-tiny items-center">
                <span className="text-default-500 font-medium flex items-center gap-1.5"><Database className="w-3 h-3" />内存</span>
                <span className="font-mono font-bold text-foreground tnum">{resources.memory_percent.toFixed(1)}%</span>
              </div>
              <Progress size="sm" value={resources.memory_percent} color={resources.memory_percent >= 85 ? 'danger' : 'success'} aria-label="内存" />
            </div>
            <div className="flex flex-col gap-1">
              <div className="flex justify-between text-tiny items-center">
                <span className="text-default-500 font-medium flex items-center gap-1.5"><HardDrive className="w-3 h-3" />录音存储</span>
                <span className="font-mono font-bold text-foreground tnum">{(100 - resources.disk_percent).toFixed(1)}%</span>
              </div>
              <Progress size="sm" value={resources.disk_percent} color={resources.disk_percent >= 90 ? 'danger' : 'primary'} aria-label="存储" />
            </div>

            <div className="text-tiny text-default-400 border-t border-divider pt-2 mt-1 flex items-center justify-between">
              <span className="flex items-center gap-1.5 font-medium">
                <Database className="w-3.5 h-3.5 text-default-400" />
                Postgres 连接池
              </span>
              <span className="font-mono text-primary font-bold tnum">{resources.db_pool_active}/{resources.db_pool_max} Active</span>
            </div>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}

/** 大 KPI 卡片：活跃通话 / 今日呼叫，带 sparkline 趋势线 */
function PrimaryKpiCard({
  label, value, unit, icon, color, trend, sparkColor, delay, hint,
}: {
  label: string; value: string; unit?: string; icon: ReactNode; color: string;
  trend?: number[]; sparkColor?: string; delay: number; hint?: string;
}) {
  return (
    <Card shadow="sm" className="dash-enter overflow-hidden relative" style={{ animationDelay: `${delay}ms` }}>
      <CardBody className="p-5 flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <span className="text-tiny font-medium text-default-500">{label}</span>
          <div className={`w-8 h-8 rounded-lg bg-content2 flex items-center justify-center ${color}`}>
            {icon}
          </div>
        </div>
        <div className="flex items-baseline gap-1">
          <span className={`text-3xl font-bold tracking-tight font-mono tnum ${color}`}>{value}</span>
          {unit && <span className="text-tiny text-default-400 font-mono">{unit}</span>}
        </div>
        {trend && trend.length >= 2 && sparkColor ? (
          <div className="mt-1 -mb-1">
            <Sparkline data={trend} color={sparkColor} />
          </div>
        ) : hint ? (
          <p className="text-[10px] text-default-400 font-medium">{hint}</p>
        ) : null}
      </CardBody>
    </Card>
  );
}

/** 小 KPI 卡片：ASR / MOS / 分机 / 中继 */
function SecondaryKpiCard({
  label, value, unit, icon, color, delay,
}: {
  label: string; value: string; unit?: string; icon: ReactNode; color: string; delay: number;
}) {
  return (
    <Card shadow="sm" className="dash-enter" style={{ animationDelay: `${delay}ms` }}>
      <CardBody className="p-4 flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <span className="text-tiny font-medium text-default-500">{label}</span>
          <span className={color}>{icon}</span>
        </div>
        <div className="flex items-baseline gap-1">
          <span className={`text-2xl font-bold tracking-tight font-mono tnum ${color}`}>{value}</span>
          {unit && <span className="text-[10px] text-default-400 font-mono">{unit}</span>}
        </div>
      </CardBody>
    </Card>
  );
}

/** Runtime 状态卡片：路由引擎、计费引擎、并发能力 */
function RuntimeStatusCard() {
  return (
    <Card shadow="sm" className="dash-enter h-full" style={{ animationDelay: '400ms' }}>
      <CardBody className="p-5 flex flex-col gap-4 justify-between">
        <div>
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center gap-2">
              <Radio className="w-4 h-4 text-success" />
              <h3 className="text-small font-bold text-foreground">Runtime 引擎</h3>
            </div>
            <Chip size="sm" variant="dot" color="success">Tokio 就绪</Chip>
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between text-tiny p-3 rounded-medium bg-success/10 font-medium">
              <span className="text-default-600 flex items-center gap-1.5">
                <CheckCircle2 className="w-3.5 h-3.5 text-success" />
                路由引擎 (LCR)
              </span>
              <Chip size="sm" color="success" variant="flat">99.99%</Chip>
            </div>
            <div className="flex items-center justify-between text-tiny p-3 rounded-medium bg-primary/10 font-medium">
              <span className="text-default-600 flex items-center gap-1.5">
                <Zap className="w-3.5 h-3.5 text-primary" />
                计费引擎
              </span>
              <Chip size="sm" color="primary" variant="flat">实时</Chip>
            </div>
          </div>
        </div>

        <div className="text-tiny text-default-400 border-t border-divider pt-3 flex items-center justify-between">
          <span className="flex items-center gap-1.5">
            <Clock className="w-3 h-3" />
            单节点最大并发
          </span>
          <span className="font-mono text-success font-bold tnum">1,700 CAPS</span>
        </div>
      </CardBody>
    </Card>
  );
}

export function DashboardPage() {
  const [data, setData] = useState<Summary>({});
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [now, setNow] = useState(() => new Date());

  const load = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    setError('');
    try {
      setData(await api.get<Summary>('/overview/summary'));
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = setInterval(() => { void load(true); }, 10000);
    const clock = setInterval(() => setNow(new Date()), 1000);
    return () => { clearInterval(timer); clearInterval(clock); };
  }, [load]);

  const totalCallsTrend = data.hourly_trends?.map(t => t.total_calls);

  return (
    <div className="flex flex-col gap-4">
      {/* Hero Banner：渐变背景 + 标题 + LIVE + 时间 + 刷新 */}
      <Card shadow="sm" className="dash-enter overflow-hidden">
        <CardBody className="p-0">
          <div className="relative bg-gradient-to-br from-primary/10 via-content1 to-content1 px-6 py-5">
            <div className="flex flex-wrap items-center justify-between gap-4 relative z-10">
              <div className="min-w-0">
                <div className="flex items-center gap-2 mb-1.5">
                  <h1 className="text-xl font-bold text-foreground tracking-tight">电信软交换运行总览</h1>
                  <Chip color="success" size="sm" variant="flat" startContent={<span className="w-2 h-2 rounded-full bg-success animate-pulse" />}>
                    LIVE
                  </Chip>
                </div>
                <p className="text-tiny text-default-500">
                  实时信令事务 · 24h 话务趋势 · QoS 媒体质量 · 集群容量监测
                </p>
              </div>
              <div className="flex items-center gap-4">
                <div className="text-right hidden sm:block">
                  <div className="text-tiny text-default-400 font-mono tnum">
                    {now.toLocaleString('zh-CN', { hour12: false })}
                  </div>
                  <div className="text-[10px] text-default-400">
                    自动刷新 · 10s
                  </div>
                </div>
                <Button
                  variant="flat"
                  size="sm"
                  isLoading={loading}
                  onPress={() => { void load(); }}
                  startContent={<RefreshCw className="w-4 h-4" />}
                >
                  刷新
                </Button>
              </div>
            </div>
            {/* 装饰性渐变光斑 */}
            <div className="absolute top-0 right-0 w-64 h-32 bg-primary/5 rounded-full blur-3xl pointer-events-none" />
            <div className="absolute bottom-0 left-1/3 w-48 h-24 bg-success/5 rounded-full blur-3xl pointer-events-none" />
          </div>
        </CardBody>
      </Card>

      {error ? (
        <ErrorState error={error} retry={() => { void load(); }} />
      ) : loading ? (
        <Card shadow="sm">
          <CardBody className="py-16 flex justify-center">
            <Spinner color="primary" label="正在拉取实时节点指标与 QoS 采样..." />
          </CardBody>
        </Card>
      ) : (
        <div className="flex flex-col gap-4">
          {/* Primary KPI：2 个大卡片，突出关键指标 */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <PrimaryKpiCard
              label="活跃通话"
              value={valueText(data.active_calls)}
              unit="calls"
              icon={<Activity className="w-4 h-4 text-success" />}
              color="text-success"
              hint="当前进行中的实时通话数"
              delay={40}
            />
            <PrimaryKpiCard
              label="今日呼叫总量"
              value={valueText(data.today_total_calls)}
              unit="calls"
              icon={<PhoneCall className="w-4 h-4 text-primary" />}
              color="text-primary"
              trend={totalCallsTrend}
              sparkColor="stroke-primary"
              delay={80}
            />
          </div>

          {/* Secondary KPI：4 个小卡片 */}
          <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
            <SecondaryKpiCard
              label="接通率 (ASR)"
              value={`${((data.answer_rate ?? 0) * 100).toFixed(2)}`}
              unit="%"
              icon={<Sparkles className="w-4 h-4 text-primary" />}
              color="text-primary"
              delay={120}
            />
            <SecondaryKpiCard
              label="平均 MOS"
              value={`${(data.avg_mos ?? 0).toFixed(2)}`}
              icon={<Award className="w-4 h-4 text-warning" />}
              color="text-warning"
              delay={160}
            />
            <SecondaryKpiCard
              label="在线分机"
              value={valueText(data.registered_users)}
              icon={<Users className="w-4 h-4 text-success" />}
              color="text-success"
              delay={200}
            />
            <SecondaryKpiCard
              label="可用中继"
              value={valueText(data.active_gateways)}
              icon={<Server className="w-4 h-4 text-primary" />}
              color="text-primary"
              delay={240}
            />
          </div>

          {/* 24h 趋势图 */}
          <HourlyTrendsSection trends={data.hourly_trends} />

          {/* 节点流量 */}
          <NodeTrafficSection />

          {/* QoS + 接通率 + Runtime 三列 */}
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            <MosQualitySection data={data} />
            <SuccessRateSection data={data} />
            <RuntimeStatusCard />
          </div>

          {/* 扩展监控：中继 + SBC + 系统资源 */}
          <MonitoringExtrasSection />
        </div>
      )}
    </div>
  );
}
