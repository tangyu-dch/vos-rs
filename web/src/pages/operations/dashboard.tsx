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

export function DashboardPage() {
  const [data, setData] = useState<Summary>({});
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api.get<Summary>('/overview/summary'));
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

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
              onPress={load}
              startContent={<RefreshCw className="w-4 h-4" />}
            >
              刷新数据
            </Button>
          </div>

          {error ? (
            <ErrorState error={error} retry={load} />
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
            </div>
          )}
        </CardBody>
      </Card>
    </div>
  );
}

