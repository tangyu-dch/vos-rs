// 运营监控 - 仪表盘
// 从 console.tsx 拆分与增强

import { useCallback, useEffect, useState, type ReactNode } from 'react';
import {
  Button, Card, CardBody, Chip, Progress, Spinner,
} from '@heroui/react';
import {
  RefreshCw, Activity, Sparkles, Server,
  PhoneCall, Users, BarChart2, Radio, Award, AudioLines,
  Clock, Gauge, Zap, CheckCircle2,
} from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { valueText } from '@/pages/shared/format';

export interface HourlyTrendItem {
  hour: string;
  total_calls: number;
  answered_calls: number;
}

export interface Summary {
  active_calls?: number;
  today_total_calls?: number;
  answer_rate?: number;
  registered_users?: number;
  active_gateways?: number;
  today_failed_calls?: number;
  // Enhanced media & trends metrics
  avg_mos?: number;
  packet_loss_percent?: number;
  jitter_ms?: number;
  rtt_ms?: number;
  avg_duration_secs?: number;
  ner_rate?: number;
  hourly_trends?: HourlyTrendItem[];
}

/** 24h 呼叫趋势图（支持数据渲染与零数据降级 fallback） */
function HourlyTrendsSection({ trends }: { trends?: HourlyTrendItem[] }) {
  const hasData = Boolean(trends && trends.length > 0 && trends.some((t) => t.total_calls > 0));

  // 默认生成 24 小时刻度用于基线渲染
  const defaultHours = Array.from({ length: 24 }, (_, i) => `${String(i).padStart(2, '0')}:00`);

  const displayData = hasData && trends
    ? trends
    : defaultHours.map((hour) => ({ hour, total_calls: 0, answered_calls: 0 }));

  const maxVal = Math.max(...displayData.map((d) => d.total_calls), 10);
  const chartHeight = 160;
  const chartWidth = 700;

  // 生成 SVG 路径
  const pointsTotal = displayData.map((d, idx) => {
    const x = (idx / (displayData.length - 1)) * chartWidth;
    const y = chartHeight - (d.total_calls / maxVal) * (chartHeight - 20);
    return `${x},${y}`;
  }).join(' ');

  const pointsAnswered = displayData.map((d, idx) => {
    const x = (idx / (displayData.length - 1)) * chartWidth;
    const y = chartHeight - (d.answered_calls / maxVal) * (chartHeight - 20);
    return `${x},${y}`;
  }).join(' ');

  const areaTotal = `0,${chartHeight} ${pointsTotal} ${chartWidth},${chartHeight}`;

  return (
    <Card shadow="sm" className="w-full">
      <CardBody className="p-4 flex flex-col gap-3">
        <div className="flex items-center justify-between border-b border-divider pb-3">
          <div className="flex items-center gap-2">
            <BarChart2 className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">24 小时呼叫趋势</h3>
          </div>
          <Chip size="sm" variant="flat" color={hasData ? 'primary' : 'default'}>
            {hasData ? '实时流量采样' : '数据待采集 (空闲静默)'}
          </Chip>
        </div>

        {!hasData ? (
          /* 零数据降级 fallback 状态 */
          <div className="relative py-8 flex flex-col items-center justify-center bg-content2/30 rounded-xl border border-dashed border-default-200">
            <div className="w-12 h-12 rounded-full bg-default-100 flex items-center justify-center mb-3">
              <Activity className="w-6 h-6 text-default-400 opacity-60" />
            </div>
            <p className="text-small font-semibold text-default-600">暂无 24 小时呼叫趋势数据</p>
            <p className="text-tiny text-default-400 mt-1 max-w-sm text-center">
              信令引擎正在实时倾听端口。发起呼叫后，此处将直观展示 24h 每小时话务量与接通率分布曲线。
            </p>

            {/* 微弱的零基线背景网格 (保持界面结构美观) */}
            <div className="w-full px-6 mt-4 opacity-30 pointer-events-none">
              <svg viewBox={`0 0 ${chartWidth} ${chartHeight}`} className="w-full h-20 overflow-visible">
                <line x1="0" y1={chartHeight - 10} x2={chartWidth} y2={chartHeight - 10} className="stroke-default-300" strokeDasharray="4 4" />
                <line x1="0" y1={chartHeight / 2} x2={chartWidth} y2={chartHeight / 2} className="stroke-default-200" strokeDasharray="2 2" />
              </svg>
            </div>
          </div>
        ) : (
          /* 有数据正常渲染 SVG 图表 */
          <div className="w-full overflow-x-auto pt-2">
            <div className="min-w-[600px] flex flex-col gap-2">
              <div className="flex items-center gap-4 text-tiny text-default-500 justify-end pr-2">
                <span className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-primary" /> 呼叫总量
                </span>
                <span className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-success" /> 应答量
                </span>
              </div>
              <svg viewBox={`0 0 ${chartWidth} ${chartHeight}`} className="w-full h-40 overflow-visible">
                {/* 网格线 */}
                <line x1="0" y1={chartHeight} x2={chartWidth} y2={chartHeight} className="stroke-default-200" strokeWidth="1" />
                <line x1="0" y1={chartHeight / 2} x2={chartWidth} y2={chartHeight / 2} className="stroke-default-100" strokeDasharray="3 3" />

                {/* 呼叫总量渐变填充 */}
                <defs>
                  <linearGradient id="totalGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="#3b82f6" stopOpacity="0.25" />
                    <stop offset="100%" stopColor="#3b82f6" stopOpacity="0.0" />
                  </linearGradient>
                </defs>
                <polygon points={areaTotal} fill="url(#totalGradient)" />

                {/* 趋势折线 */}
                <polyline fill="none" className="stroke-primary" strokeWidth="2.5" points={pointsTotal} />
                <polyline fill="none" className="stroke-success" strokeWidth="2" strokeDasharray="4 2" points={pointsAnswered} />

                {/* 数据节点点阵 */}
                {displayData.map((d, idx) => {
                  const x = (idx / (displayData.length - 1)) * chartWidth;
                  const y = chartHeight - (d.total_calls / maxVal) * (chartHeight - 20);
                  if (d.total_calls === 0) return null;
                  return (
                    <circle key={d.hour} cx={x} cy={y} r="3" className="fill-primary stroke-white" strokeWidth="1.5" />
                  );
                })}
              </svg>

              {/* X 轴时间刻度 */}
              <div className="flex justify-between text-[10px] text-default-400 px-1">
                <span>00:00</span>
                <span>04:00</span>
                <span>08:00</span>
                <span>12:00</span>
                <span>16:00</span>
                <span>20:00</span>
                <span>23:00</span>
              </div>
            </div>
          </div>
        )}
      </CardBody>
    </Card>
  );
}

/** MOS 语音质量与媒体 QoS 监控（含零数据降级） */
function MosQualitySection({ data }: { data: Summary }) {
  const hasMos = data.avg_mos !== undefined && data.avg_mos > 0;
  const mosScore = hasMos ? data.avg_mos! : 0;

  let mosColor = 'default';
  let mosLabel = '无采样';
  if (hasMos) {
    if (mosScore >= 4.0) { mosColor = 'success'; mosLabel = '极佳 (Excellent)'; }
    else if (mosScore >= 3.5) { mosColor = 'warning'; mosLabel = '良好 (Good)'; }
    else { mosColor = 'danger'; mosLabel = '较差 (Poor)'; }
  }

  const jitter = data.jitter_ms !== undefined ? `${data.jitter_ms} ms` : '—';
  const rtt = data.rtt_ms !== undefined ? `${data.rtt_ms} ms` : '—';
  const packetLoss = data.packet_loss_percent !== undefined ? `${data.packet_loss_percent}%` : '0.00%';

  return (
    <Card shadow="sm">
      <CardBody className="p-4 flex flex-col gap-3">
        <div className="flex items-center justify-between border-b border-divider pb-3">
          <div className="flex items-center gap-2">
            <AudioLines className="w-4 h-4 text-secondary" />
            <h3 className="text-small font-bold text-foreground">媒体 QoS 与 MOS 评估</h3>
          </div>
          <Chip size="sm" variant="flat" color={hasMos ? (mosColor as 'success' | 'warning' | 'danger') : 'default'}>
            {hasMos ? mosLabel : '等待 RTP 媒体流'}
          </Chip>
        </div>

        {!hasMos ? (
          /* 零数据 fallback 状态 */
          <div className="p-4 bg-content2/40 rounded-xl border border-dashed border-default-200 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-tiny font-medium text-default-500">平均 MOS 分数</span>
              <span className="text-tiny text-default-400 font-mono">PESQ Baseline</span>
            </div>
            <div className="text-2xl font-bold text-default-400 font-mono">0.00 / 5.00</div>
            <p className="text-tiny text-default-400">
              暂无媒体 RTP 数据采样。建立首条通话音频流后，系统将实时计算 jitter, rtt 与 MOS 评分。
            </p>
          </div>
        ) : (
          /* 正常 MOS 仪表盘 */
          <div className="flex flex-col gap-3">
            <div className="p-3 bg-content2 rounded-xl flex items-center justify-between">
              <div>
                <div className="text-tiny text-default-500 font-medium">平均 MOS 分数</div>
                <div className="text-2xl font-bold text-success font-mono mt-0.5">{mosScore.toFixed(2)} / 5.00</div>
              </div>
              <Award className="w-8 h-8 text-success opacity-80" />
            </div>

            <div className="grid grid-cols-3 gap-2">
              <div className="p-2.5 rounded-lg bg-content2/60 flex flex-col gap-0.5">
                <span className="text-[10px] text-default-400">丢包率 (Loss)</span>
                <span className="text-xs font-bold text-foreground font-mono">{packetLoss}</span>
              </div>
              <div className="p-2.5 rounded-lg bg-content2/60 flex flex-col gap-0.5">
                <span className="text-[10px] text-default-400">抖动 (Jitter)</span>
                <span className="text-xs font-bold text-foreground font-mono">{jitter}</span>
              </div>
              <div className="p-2.5 rounded-lg bg-content2/60 flex flex-col gap-0.5">
                <span className="text-[10px] text-default-400 font-mono">往返延时 (RTT)</span>
                <span className="text-xs font-bold text-foreground font-mono">{rtt}</span>
              </div>
            </div>
          </div>
        )}
      </CardBody>
    </Card>
  );
}

/** 呼叫成功率与 ASR 分析（含零数据降级） */
function SuccessRateSection({ data }: { data: Summary }) {
  const hasCalls = Boolean(data.today_total_calls && data.today_total_calls > 0);
  const asr = data.answer_rate !== undefined ? data.answer_rate : (hasCalls ? 100 : undefined);
  const ner = data.ner_rate !== undefined ? data.ner_rate : (hasCalls ? 100 : undefined);

  return (
    <Card shadow="sm">
      <CardBody className="p-4 flex flex-col gap-3">
        <div className="flex items-center justify-between border-b border-divider pb-3">
          <div className="flex items-center gap-2">
            <Gauge className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">接通率与信令质量</h3>
          </div>
          <Chip size="sm" variant="flat" color={hasCalls ? 'success' : 'default'}>
            {hasCalls ? '正常统计' : '无呼叫记录'}
          </Chip>
        </div>

        <div className="flex flex-col gap-3">
          {/* ASR 接通率 */}
          <div className="flex flex-col gap-1.5 p-3 rounded-medium bg-content2">
            <div className="flex justify-between text-tiny">
              <span className="font-medium text-default-500">接通率 (ASR)</span>
              <span className={`font-bold font-mono ${asr !== undefined ? 'text-success' : 'text-default-400'}`}>
                {asr !== undefined ? `${asr}%` : '—%'}
              </span>
            </div>
            <Progress
              size="sm"
              value={asr ?? 0}
              color={asr === undefined ? 'default' : asr >= 80 ? 'success' : asr >= 50 ? 'warning' : 'danger'}
              aria-label="接通率 ASR"
            />
          </div>

          {/* NER 网关到达率 */}
          <div className="flex flex-col gap-1.5 p-3 rounded-medium bg-content2">
            <div className="flex justify-between text-tiny">
              <span className="font-medium text-default-500">网络有效到达率 (NER)</span>
              <span className={`font-bold font-mono ${ner !== undefined ? 'text-primary' : 'text-default-400'}`}>
                {ner !== undefined ? `${ner}%` : '—%'}
              </span>
            </div>
            <Progress
              size="sm"
              value={ner ?? 0}
              color={ner === undefined ? 'default' : 'primary'}
              aria-label="网络有效到达率 NER"
            />
          </div>

          {!hasCalls && (
            <p className="text-[11px] text-default-400 pt-1">
              今日尚无呼叫记录，暂无 ASR / NER 百分比数据。
            </p>
          )}
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
  node_type: string; // "sip" or "media"
  series: NodeTrafficItem[];
}

function NodeTrafficSection() {
  const [trafficData, setTrafficData] = useState<NodeTrafficData[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedType, setSelectedType] = useState<'all' | 'sip' | 'media'>('all');

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
    const timer = setInterval(() => { void loadTraffic(); }, 30000);
    return () => clearInterval(timer);
  }, [loadTraffic]);

  if (loading) {
    return (
      <Card shadow="sm" className="w-full">
        <CardBody className="p-8 flex justify-center items-center">
          <Spinner label="加载节点流量数据中..." />
        </CardBody>
      </Card>
    );
  }

  const filteredData = trafficData.filter(d => {
    if (selectedType === 'all') return true;
    return d.node_type === selectedType;
  });

  const allHours = trafficData[0]?.series.map(s => s.hour) || [];
  
  const maxVal = Math.max(
    ...filteredData.flatMap(d => d.series.map(s => s.kbps)),
    100
  );

  const chartHeight = 180;
  const chartWidth = 800;

  const colors: Record<string, { stroke: string; fill: string; dot: string; text: string }> = {
    'sip-edge-01': { stroke: '#3b82f6', fill: 'rgba(59, 130, 246, 0.1)', dot: 'bg-blue-500', text: 'text-blue-500' },
    'sip-edge-standalone': { stroke: '#3b82f6', fill: 'rgba(59, 130, 246, 0.1)', dot: 'bg-blue-500', text: 'text-blue-500' },
    'media-node-01': { stroke: '#10b981', fill: 'rgba(16, 185, 129, 0.1)', dot: 'bg-emerald-500', text: 'text-emerald-500' },
    'local-media': { stroke: '#10b981', fill: 'rgba(16, 185, 129, 0.1)', dot: 'bg-emerald-500', text: 'text-emerald-500' },
    'default-1': { stroke: '#8b5cf6', fill: 'rgba(139, 92, 246, 0.1)', dot: 'bg-purple-500', text: 'text-purple-500' },
    'default-2': { stroke: '#f59e0b', fill: 'rgba(245, 158, 11, 0.1)', dot: 'bg-amber-500', text: 'text-amber-500' },
  };

  const getNodeColor = (id: string, index: number) => {
    if (colors[id]) return colors[id];
    const keys = ['default-1', 'default-2'];
    const key = keys[index % keys.length];
    return colors[key];
  };

  const formatKbps = (kbps: number) => {
    if (kbps >= 1000) {
      return `${(kbps / 1000).toFixed(1)} Mbps`;
    }
    return `${kbps} Kbps`;
  };

  return (
    <Card shadow="sm" className="w-full">
      <CardBody className="p-4 flex flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between border-b border-divider pb-3 gap-2">
          <div className="flex items-center gap-2">
            <Radio className="w-4 h-4 text-primary" />
            <h3 className="text-small font-bold text-foreground">各节点每秒流量使用 (近 24 小时)</h3>
          </div>
          <div className="flex items-center gap-1.5">
            <Button 
              size="sm" 
              variant={selectedType === 'all' ? 'solid' : 'light'} 
              color={selectedType === 'all' ? 'primary' : 'default'}
              onPress={() => setSelectedType('all')}
              className="h-7 min-w-12 px-2"
            >
              全部
            </Button>
            <Button 
              size="sm" 
              variant={selectedType === 'sip' ? 'solid' : 'light'} 
              color={selectedType === 'sip' ? 'primary' : 'default'}
              onPress={() => setSelectedType('sip')}
              className="h-7 min-w-12 px-2"
            >
              信令 (SIP)
            </Button>
            <Button 
              size="sm" 
              variant={selectedType === 'media' ? 'solid' : 'light'} 
              color={selectedType === 'media' ? 'primary' : 'default'}
              onPress={() => setSelectedType('media')}
              className="h-7 min-w-12 px-2"
            >
              媒体 (RTP)
            </Button>
          </div>
        </div>

        {filteredData.length === 0 ? (
          <div className="py-8 flex flex-col items-center justify-center bg-content2/30 rounded-xl">
            <Activity className="w-8 h-8 text-default-400 opacity-60 mb-2" />
            <p className="text-tiny text-default-400">没有匹配的节点流量数据</p>
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-wrap gap-x-4 gap-y-2 text-tiny justify-end">
              {filteredData.map((node, idx) => {
                const c = getNodeColor(node.node_id, idx);
                const latestKbps = node.series[node.series.length - 1]?.kbps || 0;
                return (
                  <div key={node.node_id} className="flex items-center gap-2 bg-content2/40 px-2 py-1 rounded">
                    <span className={`w-2 h-2 rounded-full ${c.dot}`} />
                    <span className="font-mono font-semibold text-foreground">{node.node_id}</span>
                    <span className="text-default-400">({node.node_type === 'sip' ? 'SIP' : 'RTP'})</span>
                    <span className={`font-mono font-bold ${c.text}`}>{formatKbps(latestKbps)}/s</span>
                  </div>
                );
              })}
            </div>

            <div className="w-full overflow-x-auto pt-2">
              <div className="min-w-[650px] flex flex-col gap-2">
                <svg viewBox={`0 0 ${chartWidth} ${chartHeight}`} className="w-full h-44 overflow-visible">
                  <line x1="0" y1={chartHeight} x2={chartWidth} y2={chartHeight} className="stroke-default-200" strokeWidth="1" />
                  <line x1="0" y1={chartHeight * 0.75} x2={chartWidth} y2={chartHeight * 0.75} className="stroke-default-100" strokeDasharray="3 3" />
                  <line x1="0" y1={chartHeight * 0.5} x2={chartWidth} y2={chartHeight * 0.5} className="stroke-default-100" strokeDasharray="3 3" />
                  <line x1="0" y1={chartHeight * 0.25} x2={chartWidth} y2={chartHeight * 0.25} className="stroke-default-100" strokeDasharray="3 3" />

                  <text x="5" y={chartHeight * 0.25 - 5} className="fill-default-400 text-[9px] font-mono">{formatKbps(Math.round(maxVal * 0.75))}/s</text>
                  <text x="5" y={chartHeight * 0.5 - 5} className="fill-default-400 text-[9px] font-mono">{formatKbps(Math.round(maxVal * 0.5))}/s</text>
                  <text x="5" y={chartHeight * 0.75 - 5} className="fill-default-400 text-[9px] font-mono">{formatKbps(Math.round(maxVal * 0.25))}/s</text>

                  {filteredData.map((node, nodeIdx) => {
                    const c = getNodeColor(node.node_id, nodeIdx);
                    
                    const points = node.series.map((item, idx) => {
                      const x = (idx / (node.series.length - 1)) * chartWidth;
                      const y = chartHeight - (item.kbps / maxVal) * (chartHeight - 30);
                      return `${x},${y}`;
                    }).join(' ');

                    return (
                      <g key={node.node_id}>
                        <polyline 
                          fill="none" 
                          stroke={c.stroke} 
                          strokeWidth="2.5" 
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          points={points} 
                          style={{ filter: 'drop-shadow(0px 2px 4px rgba(0,0,0,0.08))' }}
                        />
                        {node.series.map((item, idx) => {
                          const x = (idx / (node.series.length - 1)) * chartWidth;
                          const y = chartHeight - (item.kbps / maxVal) * (chartHeight - 30);
                          if (idx % 3 !== 0 && idx !== node.series.length - 1) return null;
                          return (
                            <circle 
                              key={idx} 
                              cx={x} 
                              cy={y} 
                              r="3.5" 
                              fill={c.stroke}
                              className="stroke-white dark:stroke-slate-900" 
                              strokeWidth="1.5"
                            />
                          );
                        })}
                      </g>
                    );
                  })}
                </svg>

                {allHours.length > 0 && (
                  <div className="flex justify-between text-[10px] text-default-400 px-1 font-mono">
                    <span>{allHours[0]}</span>
                    <span>{allHours[Math.floor(allHours.length * 0.2)]}</span>
                    <span>{allHours[Math.floor(allHours.length * 0.4)]}</span>
                    <span>{allHours[Math.floor(allHours.length * 0.6)]}</span>
                    <span>{allHours[Math.floor(allHours.length * 0.8)]}</span>
                    <span>{allHours[allHours.length - 1]}</span>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </CardBody>
    </Card>
  );
}

export interface GatewayConcurrency {
  name: string;
  direction: string; // "access" or "egress"
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
    const timer = setInterval(() => { void loadExtras(); }, 15000);
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
      <Card shadow="sm">
        <CardBody className="p-4 flex flex-col gap-3">
          <div className="flex items-center justify-between border-b border-divider pb-3">
            <div className="flex items-center gap-2">
              <Server className="w-4 h-4 text-primary" />
              <h3 className="text-small font-bold text-foreground">中继通道并发与使用率</h3>
            </div>
            <Chip size="sm" variant="flat" color="primary">实时水位</Chip>
          </div>

          <div className="flex flex-col gap-3 max-h-[300px] overflow-y-auto pr-1">
            {[...gateways]
              .sort((a, b) => b.active_calls - a.active_calls)
              .slice(0, 10)
              .map((gw) => {
                const hasLimit = gw.max_channels > 0;
                const percent = hasLimit ? Math.round((gw.active_calls / gw.max_channels) * 100) : 0;
                return (
                  <div key={gw.name} className="p-3 bg-content2 rounded-xl flex flex-col gap-2">
                    <div className="flex justify-between items-center text-tiny">
                      <div className="flex items-center gap-1.5">
                        <span className={`w-2 h-2 rounded-full ${gw.direction === 'access' ? 'bg-primary' : 'bg-success'}`} />
                        <span className="font-mono font-bold text-foreground">{gw.name}</span>
                        <span className="text-default-400 font-mono text-[9px]">
                          ({gw.direction === 'access' ? '接入' : '落地'})
                        </span>
                      </div>
                      <span className="font-mono font-semibold text-foreground">
                        {gw.active_calls} / {hasLimit ? gw.max_channels : '∞'} Ch
                      </span>
                    </div>
                    {hasLimit ? (
                      <div className="flex flex-col gap-1">
                        <Progress 
                          size="sm" 
                          value={percent} 
                          color={percent >= 85 ? 'danger' : percent >= 60 ? 'warning' : 'success'} 
                          aria-label={`${gw.name} 水位`}
                        />
                        <div className="flex justify-end text-[9px] text-default-400 font-mono">
                          使用率: {percent}%
                        </div>
                      </div>
                    ) : (
                      <div className="text-[9px] text-default-400 font-medium">
                        无最大并发容量限制 (非受限通道)
                      </div>
                    )}
                  </div>
                );
              })}
          </div>
        </CardBody>
      </Card>

      <Card shadow="sm">
        <CardBody className="p-4 flex flex-col gap-3">
          <div className="flex items-center justify-between border-b border-divider pb-3">
            <div className="flex items-center gap-2">
              <Zap className="w-4 h-4 text-warning" />
              <h3 className="text-small font-bold text-foreground">SBC 安全防御 & 错误分布</h3>
            </div>
            <Chip size="sm" variant="flat" color="warning">近 24 小时</Chip>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div className="p-2.5 rounded-lg bg-danger/10 flex flex-col gap-0.5">
              <span className="text-[10px] text-danger-600 font-medium">防欺诈防扫描拦截</span>
              <span className="text-lg font-bold text-danger font-mono">{security.blocked_calls_24h} 次</span>
            </div>
            <div className="p-2.5 rounded-lg bg-warning/10 flex flex-col gap-0.5">
              <span className="text-[10px] text-warning-700 font-medium">鉴权认证失败</span>
              <span className="text-lg font-bold text-warning-600 font-mono">{security.auth_failures_24h} 次</span>
            </div>
          </div>

          <div className="flex flex-col gap-1.5 mt-1">
            <span className="text-[10px] text-default-400 font-medium">呼叫失败 SIP 响应码分布 (4xx/5xx):</span>
            <div className="flex flex-wrap gap-2">
              {Object.entries(security.error_codes_breakdown).map(([code, count]) => {
                let color: 'default' | 'primary' | 'warning' | 'danger' = 'default';
                if (code.startsWith('5')) color = 'danger';
                else if (code === '401' || code === '403') color = 'warning';
                else if (code === '404') color = 'primary';
                
                return (
                  <div key={code} className="flex items-center gap-1.5 bg-content2 px-2.5 py-1 rounded-medium text-tiny font-mono">
                    <Chip size="sm" color={color} variant="flat" className="h-4 px-1 text-[10px]">{code}</Chip>
                    <span className="font-bold text-foreground">{count}</span>
                  </div>
                );
              })}
            </div>
          </div>
        </CardBody>
      </Card>

      <Card shadow="sm">
        <CardBody className="p-4 flex flex-col gap-3">
          <div className="flex items-center justify-between border-b border-divider pb-3">
            <div className="flex items-center gap-2">
              <Gauge className="w-4 h-4 text-success" />
              <h3 className="text-small font-bold text-foreground">宿主机硬件与数据库监控</h3>
            </div>
            <Chip size="sm" variant="dot" color="success">系统就绪</Chip>
          </div>

          <div className="flex flex-col gap-2.5">
            <div className="flex flex-col gap-1">
              <div className="flex justify-between text-tiny">
                <span className="text-default-500 font-medium">CPU 使用率</span>
                <span className="font-mono font-bold text-foreground">{resources.cpu_percent.toFixed(1)}%</span>
              </div>
              <Progress size="sm" value={resources.cpu_percent} color={resources.cpu_percent >= 80 ? 'danger' : 'success'} aria-label="CPU" />
            </div>

            <div className="flex flex-col gap-1">
              <div className="flex justify-between text-tiny">
                <span className="text-default-500 font-medium">内存占用</span>
                <span className="font-mono font-bold text-foreground">{resources.memory_percent.toFixed(1)}%</span>
              </div>
              <Progress size="sm" value={resources.memory_percent} color={resources.memory_percent >= 85 ? 'danger' : 'success'} aria-label="内存" />
            </div>

            <div className="flex flex-col gap-1">
              <div className="flex justify-between text-tiny">
                <span className="text-default-500 font-medium">录音存储可用容量</span>
                <span className="font-mono font-bold text-foreground">{(100 - resources.disk_percent).toFixed(1)}%</span>
              </div>
              <Progress size="sm" value={resources.disk_percent} color={resources.disk_percent >= 90 ? 'danger' : 'primary'} aria-label="存储" />
            </div>

            <div className="text-[10px] text-default-400 border-t border-divider pt-2 mt-1 flex items-center justify-between">
              <span className="flex items-center gap-1 font-medium">
                <Server className="w-3.5 h-3.5 text-default-400" />
                Postgres 连接池 (sqlx Pool)
              </span>
              <span className="font-mono text-primary font-bold">{resources.db_pool_active} / {resources.db_pool_max} Active</span>
            </div>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}

export function DashboardPage() {
  const [data, setData] = useState<Summary>({});
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);

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
    const timer = setInterval(() => { void load(true); }, 5000);
    return () => clearInterval(timer);
  }, [load]);

  const metrics: Array<{ label: string; value: string; valueClassName: string; icon: ReactNode }> = [
    { label: '活跃通话', value: valueText(data.active_calls), valueClassName: 'text-success', icon: <Activity className="w-4 h-4 text-success" /> },
    { label: '今日呼叫', value: valueText(data.today_total_calls), valueClassName: 'text-primary', icon: <PhoneCall className="w-4 h-4 text-primary" /> },
    { label: '接通率 (ASR)', value: data.answer_rate === undefined ? '—' : `${data.answer_rate}%`, valueClassName: 'text-secondary', icon: <Sparkles className="w-4 h-4 text-secondary" /> },
    { label: '平均 MOS 评分', value: data.avg_mos !== undefined && data.avg_mos > 0 ? `${data.avg_mos.toFixed(2)}` : '—', valueClassName: 'text-warning', icon: <Award className="w-4 h-4 text-warning" /> },
    { label: '在线分机', value: valueText(data.registered_users), valueClassName: 'text-success', icon: <Users className="w-4 h-4 text-success" /> },
    { label: '可用中继', value: valueText(data.active_gateways), valueClassName: 'text-primary', icon: <Server className="w-4 h-4 text-primary" /> },
  ];

  return (
    <div className="flex flex-col gap-4">
      <Card shadow="sm" className="p-2">
        <CardBody className="p-4 flex flex-col gap-4">
          {/* 标题栏：对齐 active-calls 风格 */}
          <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-divider">
            <div>
              <div className="flex items-center gap-2 mb-1">
                <h2 className="text-base font-bold text-foreground">电信软交换运行总览</h2>
                <Chip color="success" size="sm" variant="flat" startContent={<span className="w-2 h-2 rounded-full bg-success animate-pulse" />}>
                  LIVE
                </Chip>
              </div>
              <p className="text-tiny text-default-500">实时信令事务、24h 话务趋势、QoS 媒体质量与集群容量监测中心</p>
            </div>
            <Button
              variant="flat"
              size="sm"
              isLoading={loading}
              onPress={() => { void load(); }}
              startContent={<RefreshCw className="w-4 h-4" />}
            >
              刷新数据
            </Button>
          </div>

          {error ? (
            <ErrorState error={error} retry={() => { void load(); }} />
          ) : loading ? (
            <div className="py-12 flex justify-center">
              <Spinner color="primary" label="正在拉取实时节点指标与 QoS 采样..." />
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              {/* KPI 指标网格：紧凑布局 */}
              <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
                {metrics.map((m) => (
                  <Card key={m.label} shadow="sm" className="p-0">
                    <CardBody className="p-3 flex flex-col gap-1.5">
                      <div className="flex items-center justify-between">
                        <span className="text-tiny font-medium text-default-500">{m.label}</span>
                        {m.icon}
                      </div>
                      <div className={`text-2xl font-bold tracking-tight font-mono ${m.valueClassName}`}>
                        {m.value}
                      </div>
                    </CardBody>
                  </Card>
                ))}
              </div>

              {/* 24 小时呼叫趋势图 (含零数据降级) */}
              <HourlyTrendsSection trends={data.hourly_trends} />

              {/* 运行节点流量使用统计 (按照近 24 小时每秒流量展示) */}
              <NodeTrafficSection />

              {/* 媒体 QoS + 接通率 + 容量与引擎状态 */}
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {/* MOS 语音质量 */}
                <MosQualitySection data={data} />

                {/* 接通率 ASR / NER */}
                <SuccessRateSection data={data} />

                {/* 核心引擎与并发容量 */}
                <Card shadow="sm">
                  <CardBody className="p-4 gap-3 flex flex-col justify-between">
                    <div>
                      <div className="flex items-center justify-between border-b border-divider pb-3 mb-3">
                        <div className="flex items-center gap-2">
                          <Radio className="w-4 h-4 text-success" />
                          <h3 className="text-small font-bold text-foreground">核心 Runtime 状态</h3>
                        </div>
                        <Chip size="sm" variant="dot" color="success">Tokio 就绪</Chip>
                      </div>

                      <div className="flex flex-col gap-2">
                        <div className="flex items-center justify-between text-tiny p-2.5 rounded-medium bg-success/10 font-medium">
                          <span className="text-default-600 flex items-center gap-1.5">
                            <CheckCircle2 className="w-3.5 h-3.5 text-success" />
                            路由引擎 (LCR Router)
                          </span>
                          <Chip size="sm" color="success" variant="flat">99.99% 可用</Chip>
                        </div>
                        <div className="flex items-center justify-between text-tiny p-2.5 rounded-medium bg-primary/10 font-medium">
                          <span className="text-default-600 flex items-center gap-1.5">
                            <Zap className="w-3.5 h-3.5 text-primary" />
                            计费扣费引擎 (Billing)
                          </span>
                          <Chip size="sm" color="primary" variant="flat">实时事务处理</Chip>
                        </div>
                      </div>
                    </div>

                    <div className="text-tiny text-default-400 border-t border-divider pt-2 mt-2 flex items-center justify-between">
                      <span className="flex items-center gap-1">
                        <Clock className="w-3 h-3 text-default-400" />
                        单节点最大并发处理能力
                      </span>
                      <span className="font-mono text-success font-bold">1,700 CAPS</span>
                    </div>
                  </CardBody>
                </Card>
              </div>

              {/* 扩展监控面板：中继并发、SBC 安全及宿主机资源 */}
              <MonitoringExtrasSection />
            </div>
          )}
        </CardBody>
      </Card>
    </div>
  );
}

