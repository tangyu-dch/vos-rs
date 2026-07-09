import { useEffect, useState, useRef, useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { Spin, Alert, Empty } from '@arco-design/web-react';
import {
  IconRefresh,
  IconNotification,
  IconFullscreen,
  IconSettings,
  IconDashboard,
  IconPhone,
  IconUserGroup,
  IconCommon,
  IconStorage,
  IconShareAlt,
  IconFile,
} from '@arco-design/web-react/icon';
import * as echarts from 'echarts';
import { apiService } from '@/services/api';
import type { DashboardStats, HourlyTrend, ActiveCall } from '@/types';
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

// ─── Sidebar Nav Items ───
interface SidebarItem {
  key: string;
  icon: React.ReactNode;
  title: string;
  group: string;
  badge?: number;
}

const SIDEBAR_ITEMS: SidebarItem[] = [
  { key: '/dashboard', icon: <IconDashboard />, title: '全局概览', group: '实时监控' },
  { key: '/active-calls', icon: <IconPhone />, title: '活跃呼叫', group: '实时监控' },
  { key: '/users', icon: <IconUserGroup />, title: 'SIP 用户', group: '号码路由' },
  { key: '/gateways', icon: <IconStorage />, title: '落地网关', group: '号码路由' },
  { key: '/peer-gateways', icon: <IconShareAlt />, title: '对接网关', group: '号码路由' },
  { key: '/routes', icon: <IconFile />, title: '路由管理', group: '号码路由' },
  { key: '/registrations', icon: <IconCommon />, title: '注册信息', group: '号码路由' },
  { key: '/cdr', icon: <IconFile />, title: '呼叫记录', group: '数据分析' },
  { key: '/reports', icon: <IconCommon />, title: '报表分析', group: '数据分析' },
  { key: '/settings', icon: <IconSettings />, title: '系统设置', group: '系统' },
];

const SIDEBAR_GROUPS = ['实时监控', '号码路由', '数据分析', '系统'];

export default function Dashboard() {
  const navigate = useNavigate();
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [trend, setTrend] = useState<HourlyTrend[]>([]);
  const [activeCalls, setActiveCalls] = useState<ActiveCall[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [callFilter, setCallFilter] = useState<CallFilter>('all');
  const [trendPeriod, setTrendPeriod] = useState<'today' | 'week' | 'month'>('today');
  const [currentTime, setCurrentTime] = useState(new Date());
  const [now, setNow] = useState(Date.now());

  const trendRef = useRef<HTMLDivElement>(null);
  const trendChart = useRef<echarts.ECharts | null>(null);
  const donutRef = useRef<HTMLDivElement>(null);
  const donutChart = useRef<echarts.ECharts | null>(null);

  // ─── Clock ───
  useEffect(() => {
    const t = setInterval(() => setCurrentTime(new Date()), 1000);
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => { clearInterval(t); clearInterval(tick); };
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
      trendChart.current = echarts.init(trendRef.current);
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
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
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
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
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
      donutChart.current = echarts.init(donutRef.current);
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

  const timestamp = `${currentTime.getFullYear()}-${String(currentTime.getMonth() + 1).padStart(2, '0')}-${String(currentTime.getDate()).padStart(2, '0')} ${String(currentTime.getHours()).padStart(2, '0')}:${String(currentTime.getMinutes()).padStart(2, '0')}:${String(currentTime.getSeconds()).padStart(2, '0')}`;

  if (loading) {
    return (
      <div className="loading-wrap">
        <Spin size={32} />
        <span>正在加载监控数据...</span>
      </div>
    );
  }

  return (
    <div className="callcenter-layout">
      {/* ─── Sidebar ─── */}
      <aside className="cc-sidebar">
        <div className="cc-sidebar__brand">
          <div className="cc-sidebar__logo">
            <svg width="24" height="24" viewBox="0 0 32 32" fill="none">
              <rect width="32" height="32" rx="8" fill="url(#sb-grad)" />
              <path d="M9 11.5a3.5 3.5 0 0 1 3.5-3.5h7A3.5 3.5 0 0 1 23 11.5v9A3.5 3.5 0 0 1 19.5 24h-7A3.5 3.5 0 0 1 9 20.5v-9Z" stroke="#fff" strokeWidth="1.8" />
              <circle cx="16" cy="16" r="2.4" fill="#fff" />
              <path d="M16 9v3M16 20v3M9 16h3M20 16h3" stroke="#fff" strokeWidth="1.8" strokeLinecap="round" />
              <defs><linearGradient id="sb-grad" x1="0" y1="0" x2="32" y2="32"><stop stopColor="#4080FF" /><stop offset="1" stopColor="#0FC6C2" /></linearGradient></defs>
            </svg>
          </div>
          <div className="cc-sidebar__brand-text">
            <span className="cc-sidebar__brand-name">VOS-RS</span>
            <span className="cc-sidebar__brand-sub">VoIP 运营平台</span>
          </div>
        </div>

        <nav className="cc-sidebar__nav">
          {SIDEBAR_GROUPS.map((g) => (
            <div className="cc-sidebar__group" key={g}>
              <div className="cc-sidebar__group-title">{g}</div>
              {SIDEBAR_ITEMS.filter((it) => it.group === g).map((item) => (
                <div
                  key={item.key}
                  className={`cc-sidebar__item ${item.key === '/dashboard' ? 'is-active' : ''}`}
                  onClick={() => navigate(item.key)}
                >
                  <span className="cc-sidebar__icon">{item.icon}</span>
                  <span className="cc-sidebar__title">{item.title}</span>
                  {item.badge && <span className="cc-sidebar__badge">{item.badge}</span>}
                </div>
              ))}
            </div>
          ))}
        </nav>

        <div className="cc-sidebar__footer">
          <div className="cc-sidebar__user">
            <div className="cc-sidebar__user-avatar">A</div>
            <div className="cc-sidebar__user-info">
              <span className="cc-sidebar__user-name">Admin</span>
              <span className="cc-sidebar__user-role">超级管理员</span>
            </div>
          </div>
        </div>
      </aside>

      {/* ─── Main Content ─── */}
      <div className="cc-main">
        {/* Header */}
        <header className="cc-header">
          <div className="cc-header__left">
            <h1 className="cc-header__title">全局概览</h1>
            <span className="cc-header__time">● {timestamp}</span>
          </div>
          <div className="cc-header__right">
            <button className="cc-header__btn" onClick={loadData} title="刷新">
              <IconRefresh />
            </button>
            <button className="cc-header__btn cc-header__btn--badge" title="通知">
              <IconNotification />
              <span className="cc-header__badge">5</span>
            </button>
            <button className="cc-header__btn" title="全屏">
              <IconFullscreen />
            </button>
            <div className="theme-toggle" onClick={() => {
              const html = document.documentElement;
              const current = html.getAttribute('data-theme');
              const next = current === 'dark' ? 'light' : 'dark';
              html.setAttribute('data-theme', next);
              document.querySelector('.app')?.setAttribute('data-theme', next);
              localStorage.setItem('vos-theme', next);
            }} title="切换主题">
              <span className="theme-toggle__knob">
                <span>{document.documentElement.getAttribute('data-theme') === 'dark' ? '🌙' : '☀️'}</span>
              </span>
            </div>
            <div className="topbar-avatar">A</div>
          </div>
        </header>

        {/* Content */}
        <main className="cc-content">
          {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

          {/* KPI Cards */}
          <div className="cc-kpi-grid">
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
            {/* Traffic Trend */}
            <div className="cc-chart-card">
              <div className="cc-chart-card__header">
                <h3 className="cc-chart-card__title">话务量趋势</h3>
                <div className="cc-chart-card__tabs">
                  {(['today', 'week', 'month'] as const).map((p) => (
                    <button
                      key={p}
                      className={`cc-tab-btn ${trendPeriod === p ? 'cc-tab-btn--active' : ''}`}
                      onClick={() => setTrendPeriod(p)}
                    >
                      {p === 'today' ? '今日' : p === 'week' ? '本周' : '本月'}
                    </button>
                  ))}
                </div>
              </div>
              <div ref={trendRef} className="cc-chart" />
              {trend.length === 0 && (
                <div className="cc-chart-empty">
                  <Empty description="今日暂无呼叫数据" />
                </div>
              )}
            </div>

            {/* Call State Distribution */}
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
        </main>
      </div>
    </div>
  );
}
