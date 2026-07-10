import { useEffect, useState, useRef, useCallback } from 'react';
import { Spin, Alert, Empty } from '@arco-design/web-react';
import { IconRefresh } from '@arco-design/web-react/icon';
import { apiService } from '@/services/api';
import type { DashboardStats, HourlyTrend, ActiveCall } from '@/types';
import { graphic, init, type ECharts } from '@/utils/charts';
import './Dashboard.css';

// ─── Helpers ───
function formatTime(durationMs: number): string {
  const seconds = Math.max(0, Math.floor(durationMs / 1000));
  const m = String(Math.floor(seconds / 60)).padStart(2, '0');
  const s = String(seconds % 60).padStart(2, '0');
  return `${m}:${s}`;
}

function extractSipUser(uri?: string): string {
  if (!uri) return '--';
  const match = uri.match(/sip:([^@]+)/);
  return match ? match[1] : uri;
}

function getThemeColors() {
  const s = getComputedStyle(document.documentElement);
  return {
    text: s.getPropertyValue('--text-primary').trim() || '#f0f1f5',
    textSec: s.getPropertyValue('--text-secondary').trim() || '#a0a3b5',
    muted: s.getPropertyValue('--text-muted').trim() || '#5c5f72',
    border: s.getPropertyValue('--border-subtle').trim() || 'rgba(255,255,255,0.06)',
    panel1: s.getPropertyValue('--bg-panel-1').trim() || '#0f1117',
    panel2: s.getPropertyValue('--bg-panel-2').trim() || '#161821',
  };
}

// ─── State Config ───
const STATE_CONFIG: Record<string, { color: string; text: string; dot: string }> = {
  Routing: { color: 'var(--status-break)', text: '路由中', dot: 'var(--status-break)' },
  Ringing: { color: 'var(--status-break)', text: '振铃中', dot: 'var(--status-break)' },
  Established: { color: 'var(--status-online)', text: '通话中', dot: 'var(--status-online)' },
  Terminated: { color: 'var(--text-muted)', text: '已结束', dot: 'var(--text-muted)' },
};

type CallFilter = 'all' | 'Established' | 'Ringing' | 'Routing';

// ─── KPI Card ───
interface KpiCardProps {
  label: string;
  value: string | number;
  trend?: { value: string; direction: 'up' | 'down' | 'flat' };
  sub: string;
  barColor?: string;
  barPercent?: number;
}

function KpiCard({ label, value, trend, sub, barColor = 'var(--accent)', barPercent = 60 }: KpiCardProps) {
  return (
    <div className="kpi-card">
      <div className="kpi-card__header">
        <span className="kpi-card__label">{label}</span>
        {trend && (
          <span className={`kpi-card__trend kpi-card__trend--${trend.direction}`}>
            {trend.direction === 'up' && '↑'}
            {trend.direction === 'down' && '↓'}
            {trend.direction === 'flat' && '~'}
            {trend.value}
          </span>
        )}
      </div>
      <div className="kpi-card__value">{value}</div>
      <div className="kpi-card__sub">{sub}</div>
      <div className="kpi-card__bar">
        <div className="kpi-card__bar-fill" style={{ width: `${barPercent}%`, background: barColor }} />
      </div>
    </div>
  );
}

export default function Dashboard() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [trend, setTrend] = useState<HourlyTrend[]>([]);
  const [activeCalls, setActiveCalls] = useState<ActiveCall[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [callFilter, setCallFilter] = useState<CallFilter>('all');
  const [now, setNow] = useState(Date.now());

  const trendRef = useRef<HTMLDivElement>(null);
  const trendChart = useRef<ECharts | null>(null);
  const donutRef = useRef<HTMLDivElement>(null);
  const donutChart = useRef<ECharts | null>(null);

  // ─── Clock ───
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(tick);
  }, []);

  // ─── Load Data ───
  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [s, t, c] = await Promise.all([
        apiService.getDashboardStats(),
        apiService.getHourlyTrend(),
        apiService.getActiveCalls(),
      ]);
      setStats(s);
      setTrend(t);
      setActiveCalls(c);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载数据失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
    const refreshInterval = setInterval(loadData, 5000);
    return () => clearInterval(refreshInterval);
  }, [loadData]);

  // ─── Trend Chart ───
  useEffect(() => {
    if (!trendRef.current || trend.length === 0) return;
    if (!trendChart.current) {
      trendChart.current = init(trendRef.current);
    }
    const tc = getThemeColors();
    const hours = trend.map((t) => `${String(t.hour).padStart(2, '0')}:00`);
    const totals = trend.map((t) => t.total);
    const answered = trend.map((t) => t.answered);

    trendChart.current.setOption({
      tooltip: { trigger: 'axis', backgroundColor: tc.panel2, borderColor: tc.border, textStyle: { color: tc.text } },
      legend: { data: ['呼入', '呼出'], bottom: 0, icon: 'roundRect', textStyle: { color: tc.textSec } },
      grid: { left: 50, right: 20, top: 20, bottom: 40 },
      xAxis: {
        type: 'category',
        data: hours,
        boundaryGap: false,
        axisLine: { lineStyle: { color: tc.border } },
        axisLabel: { color: tc.muted, fontSize: 11 },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: tc.border, type: 'dashed' } },
        axisLabel: { color: tc.muted, fontSize: 11 },
      },
      series: [
        {
          name: '呼入',
          type: 'line',
          smooth: true,
          symbol: 'circle',
          symbolSize: 6,
          data: totals,
          lineStyle: { color: '#60a5fa', width: 2.5 },
          itemStyle: { color: '#60a5fa' },
          areaStyle: {
            color: new graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(96,165,250,0.25)' },
              { offset: 1, color: 'rgba(96,165,250,0.01)' },
            ]),
          },
        },
        {
          name: '呼出',
          type: 'line',
          smooth: true,
          symbol: 'circle',
          symbolSize: 6,
          data: answered,
          lineStyle: { color: '#34d399', width: 2.5 },
          itemStyle: { color: '#34d399' },
          areaStyle: {
            color: new graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(52,211,153,0.22)' },
              { offset: 1, color: 'rgba(52,211,153,0.01)' },
            ]),
          },
        },
      ],
    });
  }, [trend]);

  // ─── Donut Chart ───
  useEffect(() => {
    if (!donutRef.current) return;
    if (!donutChart.current) {
      donutChart.current = init(donutRef.current);
    }
    const tc = getThemeColors();

    const stateCounts: Record<string, number> = { Established: 0, Ringing: 0, Routing: 0 };
    activeCalls.forEach((c) => {
      stateCounts[c.state] = (stateCounts[c.state] || 0) + 1;
    });
    const total = activeCalls.length || 1;

    donutChart.current.setOption({
      tooltip: { trigger: 'item', backgroundColor: tc.panel2, borderColor: tc.border, textStyle: { color: tc.text } },
      legend: {
        orient: 'vertical',
        right: '5%',
        top: 'center',
        icon: 'circle',
        itemWidth: 10,
        itemHeight: 10,
        textStyle: { color: tc.textSec, fontSize: 12 },
        formatter: (name: string) => {
          const map: Record<string, { count: number; percent: string }> = {
            '通话中': { count: stateCounts.Established, percent: `${((stateCounts.Established / total) * 100).toFixed(0)}%` },
            '振铃中': { count: stateCounts.Ringing, percent: `${((stateCounts.Ringing / total) * 100).toFixed(0)}%` },
            '路由中': { count: stateCounts.Routing, percent: `${((stateCounts.Routing / total) * 100).toFixed(0)}%` },
          };
          const info = map[name];
          return info ? `${name} ${info.count} (${info.percent})` : name;
        },
      },
      series: [
        {
          type: 'pie',
          radius: ['52%', '75%'],
          center: ['35%', '50%'],
          avoidLabelOverlap: true,
          itemStyle: { borderRadius: 4, borderColor: tc.panel1, borderWidth: 2 },
          label: { show: false },
          emphasis: { scaleSize: 4 },
          data: [
            { value: stateCounts.Established, name: '通话中', itemStyle: { color: '#60a5fa' } },
            { value: stateCounts.Ringing, name: '振铃中', itemStyle: { color: '#fbbf24' } },
            { value: stateCounts.Routing, name: '路由中', itemStyle: { color: '#34d399' } },
          ],
        },
      ],
      graphic: [
        {
          type: 'text',
          left: '28%',
          top: '42%',
          style: {
            text: String(activeCalls.length),
            fontSize: 28,
            fontWeight: 700,
            fill: tc.text,
            fontFamily: 'Outfit, sans-serif',
            textAlign: 'center',
          },
        },
        {
          type: 'text',
          left: '28.5%',
          top: '54%',
          style: {
            text: '活跃呼叫',
            fontSize: 12,
            fill: tc.muted,
            textAlign: 'center',
          },
        },
      ],
    });
  }, [activeCalls]);

  // ─── Resize ───
  useEffect(() => {
    const onResize = () => {
      trendChart.current?.resize();
      donutChart.current?.resize();
    };
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
      trendChart.current?.dispose();
      donutChart.current?.dispose();
    };
  }, []);

  // ─── Call Filter ───
  const filteredCalls = activeCalls.filter((c) => callFilter === 'all' || c.state === callFilter);

  // ─── Derived KPI Values ───
  const totalCalls = stats?.today_total_calls ?? 0;
  const answeredCalls = stats?.today_answered_calls ?? 0;
  const failedCalls = stats?.today_failed_calls ?? 0;
  const activeCallsCount = stats?.active_calls ?? activeCalls.length;
  const answerRate = stats?.answer_rate ?? 0;
  const registeredUsers = stats?.registered_users ?? 0;

  if (loading) {
    return (
      <div className="loading-wrap">
        <Spin size={32} />
        <span>正在加载监控数据...</span>
      </div>
    );
  }

  return (
    <div className="page-wrap dashboard">
      {/* Header */}
      <div className="page-header">
        <div className="page-header__title">
          <h1>全局概览</h1>
          <span className="sub">实时掌握平台呼叫、质量与设备状态</span>
        </div>
        <div className="page-header__actions">
          <span className="live-indicator">
            <span className="live-indicator__dot" />
            {activeCalls.length} 通进行中
          </span>
          <button className="section-btn" onClick={loadData}>
            <IconRefresh style={{ marginRight: 4 }} />
            刷新
          </button>
        </div>
      </div>

      {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

      {/* KPI Cards */}
      <div className="kpi-grid">
        <KpiCard
          label="当前活跃呼叫"
          value={activeCallsCount}
          trend={{ value: `${activeCalls.length} 通实时`, direction: 'up' }}
          sub={`已接通 ${answeredCalls} · 失败 ${failedCalls}`}
          barColor="var(--accent)"
          barPercent={Math.min(100, activeCallsCount * 2)}
        />
        <KpiCard
          label="今日总话务量"
          value={totalCalls.toLocaleString()}
          trend={{ value: `${answerRate.toFixed(1)}% 接通率`, direction: answerRate >= 80 ? 'up' : 'down' }}
          sub={`呼入 ${answeredCalls.toLocaleString()} · 呼出 ${failedCalls.toLocaleString()}`}
          barColor="#60a5fa"
          barPercent={Math.min(100, totalCalls / 100)}
        />
        <KpiCard
          label="注册终端"
          value={registeredUsers}
          sub={`${stats?.active_gateways || 0} 个网关在线`}
          barColor="var(--status-online)"
          barPercent={Math.min(100, registeredUsers)}
        />
        <KpiCard
          label="平均 MOS"
          value={stats?.avg_mos ? stats.avg_mos.toFixed(2) : '--'}
          sub={stats?.avg_mos ? (stats.avg_mos >= 4 ? '语音质量优秀' : '语音质量良好') : '暂无数据'}
          barColor={stats?.avg_mos && stats.avg_mos >= 4 ? 'var(--status-online)' : 'var(--status-break)'}
          barPercent={stats?.avg_mos ? (stats.avg_mos / 5) * 100 : 0}
        />
      </div>

      {/* Charts Row */}
      <div className="cc-charts-row">
        <div className="cc-chart-card">
          <div className="cc-chart-card__header">
            <h3 className="cc-chart-card__title">话务量趋势</h3>
          </div>
          <div ref={trendRef} className="cc-chart" />
          {trend.length === 0 && (
            <div className="cc-chart-empty">
              <Empty description="今日暂无呼叫数据" />
            </div>
          )}
        </div>

        <div className="cc-chart-card cc-chart-card--donut">
          <div className="cc-chart-card__header">
            <h3 className="cc-chart-card__title">呼叫状态分布</h3>
          </div>
          <div ref={donutRef} className="cc-chart cc-chart--donut" />
          {activeCalls.length === 0 && (
            <div className="cc-chart-empty">
              <Empty description="当前无活跃呼叫" />
            </div>
          )}
        </div>
      </div>

      {/* Active Calls Table */}
      <div className="cc-table-card">
        <div className="cc-table-card__header">
          <h3 className="cc-table-card__title">活跃呼叫实时状态</h3>
          <div className="cc-table-card__filters">
            {(['all', 'Established', 'Ringing', 'Routing'] as const).map((f) => (
              <button
                key={f}
                className={`cc-filter-btn ${callFilter === f ? 'cc-filter-btn--active' : ''}`}
                onClick={() => setCallFilter(f)}
              >
                {f === 'all' ? '全部' : STATE_CONFIG[f]?.text || f}
              </button>
            ))}
          </div>
        </div>

        <div className="cc-table-wrap">
          <table className="cc-table">
            <thead>
              <tr>
                <th>呼叫 ID</th>
                <th>主叫</th>
                <th>被叫</th>
                <th>状态</th>
                <th>通话时长</th>
                <th>网关</th>
              </tr>
            </thead>
            <tbody>
              {filteredCalls.map((call) => {
                const stateCfg = STATE_CONFIG[call.state] || { color: 'var(--text-muted)', text: call.state, dot: 'var(--text-muted)' };
                return (
                  <tr key={call.call_id}>
                    <td>
                      <span className="cc-agent-id">{call.call_id.substring(0, 16)}...</span>
                    </td>
                    <td>
                      <span className="mono">{extractSipUser(call.caller)}</span>
                    </td>
                    <td>
                      <span className="mono">{extractSipUser(call.callee)}</span>
                    </td>
                    <td>
                      <span className="cc-state-tag" style={{ color: stateCfg.color }}>
                        <span className="cc-state-dot" style={{ background: stateCfg.dot }} />
                        {stateCfg.text}
                      </span>
                    </td>
                    <td>
                      <span className="cc-duration mono" style={{ color: call.state === 'Established' ? 'var(--status-online)' : 'var(--text-muted)' }}>
                        {call.started_at_ms ? formatTime(now - call.started_at_ms) : '--'}
                      </span>
                    </td>
                    <td>
                      <span className="mono">{call.gateway || '--'}</span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {filteredCalls.length === 0 && (
            <div className="cc-table-empty">
              <Empty description="当前无活跃呼叫" />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
