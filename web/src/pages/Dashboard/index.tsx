import { useEffect, useState, useRef, useCallback } from 'react';
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
import type { DashboardStats, HourlyTrend } from '@/types';
import './Dashboard.css';

// ─── Helpers ───
function formatTime(durationSec: number): string {
  const m = String(Math.floor(durationSec / 60)).padStart(2, '0');
  const s = String(durationSec % 60).padStart(2, '0');
  return `${m}:${s}`;
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
    accent: s.getPropertyValue('--accent').trim() || '#3ee8c8',
    danger: s.getPropertyValue('--color-danger').trim() || '#f25c78',
    warning: s.getPropertyValue('--status-break').trim() || '#f5a623',
  };
}

// ─── Mock Agent Data (will be replaced with real API) ───
interface AgentStatus {
  id: string;
  name: string;
  agentId: string;
  skillGroup: string;
  state: 'talking' | 'idle' | 'ringing' | 'wrapup' | 'offline';
  currentCallDuration: number;
  todayAnswered: number;
  todayOutbound: number;
  utilization: number;
  satisfaction: number;
}

const MOCK_AGENTS: AgentStatus[] = [
  { id: '1', name: '李明辉', agentId: '10086', skillGroup: '售前咨询', state: 'talking', currentCallDuration: 204, todayAnswered: 38, todayOutbound: 12, utilization: 87, satisfaction: 4 },
  { id: '2', name: '王思琪', agentId: '10023', skillGroup: '售后服务', state: 'idle', currentCallDuration: 0, todayAnswered: 45, todayOutbound: 8, utilization: 92, satisfaction: 5 },
  { id: '3', name: '张伟', agentId: '10051', skillGroup: '技术支持', state: 'ringing', currentCallDuration: 8, todayAnswered: 29, todayOutbound: 15, utilization: 73, satisfaction: 3 },
  { id: '4', name: '陈小红', agentId: '10007', skillGroup: '售前咨询', state: 'wrapup', currentCallDuration: 22, todayAnswered: 51, todayOutbound: 3, utilization: 95, satisfaction: 5 },
  { id: '5', name: '赵刚', agentId: '10099', skillGroup: '技术支持', state: 'offline', currentCallDuration: 0, todayAnswered: 0, todayOutbound: 0, utilization: 0, satisfaction: 0 },
];

const STATE_CONFIG: Record<string, { color: string; text: string; dot: string }> = {
  talking: { color: 'var(--status-online)', text: '通话中', dot: 'var(--status-online)' },
  idle: { color: 'var(--accent)', text: '空闲就绪', dot: 'var(--accent)' },
  ringing: { color: 'var(--status-break)', text: '振铃中', dot: 'var(--status-break)' },
  wrapup: { color: 'var(--color-info)', text: '话后处理', dot: 'var(--color-info)' },
  offline: { color: 'var(--text-muted)', text: '未上线', dot: 'var(--text-muted)' },
};

const SKILL_COLORS: Record<string, string> = {
  '售前咨询': 'var(--status-online)',
  '售后服务': 'var(--accent)',
  '技术支持': 'var(--status-break)',
};

type AgentFilter = 'all' | 'talking' | 'idle' | 'wrapup' | 'offline';

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
  { key: '/agent-monitor', icon: <IconUserGroup />, title: '坐席监控', group: '实时监控', badge: 3 },
  { key: '/queue-monitor', icon: <IconPhone />, title: '排队监控', group: '实时监控' },
  { key: '/realtime-traffic', icon: <IconCommon />, title: '实时话务', group: '实时监控' },
  { key: '/queue-manage', icon: <IconPhone />, title: '队列管理', group: '运营管理' },
  { key: '/agent-manage', icon: <IconUserGroup />, title: '坐席管理', group: '运营管理' },
  { key: '/schedule', icon: <IconFile />, title: '排班管理', group: '运营管理' },
  { key: '/reports', icon: <IconStorage />, title: '报表中心', group: '数据分析' },
  { key: '/quality', icon: <IconShareAlt />, title: '质检管理', group: '数据分析' },
  { key: '/settings', icon: <IconSettings />, title: '系统设置', group: '系统' },
];

const SIDEBAR_GROUPS = ['实时监控', '运营管理', '数据分析', '系统'];

export default function Dashboard() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [trend, setTrend] = useState<HourlyTrend[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [agentFilter, setAgentFilter] = useState<AgentFilter>('all');
  const [trendPeriod, setTrendPeriod] = useState<'today' | 'week' | 'month'>('today');
  const [queueFilter, setQueueFilter] = useState<'all' | 'skill'>('all');
  const [sidebarCollapsed] = useState(false);
  const [currentTime, setCurrentTime] = useState(new Date());

  const trendRef = useRef<HTMLDivElement>(null);
  const trendChart = useRef<echarts.ECharts | null>(null);
  const donutRef = useRef<HTMLDivElement>(null);
  const donutChart = useRef<echarts.ECharts | null>(null);

  // ─── Clock ───
  useEffect(() => {
    const t = setInterval(() => setCurrentTime(new Date()), 1000);
    return () => clearInterval(t);
  }, []);

  // ─── Load Data ───
  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [s, t] = await Promise.all([
        apiService.getDashboardStats(),
        apiService.getHourlyTrend(),
      ]);
      setStats(s);
      setTrend(t);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载数据失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
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
    if (!donutRef.current || !stats) return;
    if (!donutChart.current) {
      donutChart.current = echarts.init(donutRef.current);
    }
    const talking = stats.active_calls || 52;
    const idle = 24;
    const wrapup = 10;
    const breakaway = 14;
    const offline = 20;
    const total = talking + idle + wrapup + breakaway + offline;

    const tc = getThemeColors();
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
            '通话中': { count: talking, percent: `${((talking / total) * 100).toFixed(0)}%` },
            '空闲就绪': { count: idle, percent: `${((idle / total) * 100).toFixed(0)}%` },
            '后处理': { count: wrapup, percent: `${((wrapup / total) * 100).toFixed(0)}%` },
            '小休/离席': { count: breakaway, percent: `${((breakaway / total) * 100).toFixed(0)}%` },
            '未上线': { count: offline, percent: `${((offline / total) * 100).toFixed(0)}%` },
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
            { value: talking, name: '通话中', itemStyle: { color: '#60a5fa' } },
            { value: idle, name: '空闲就绪', itemStyle: { color: '#34d399' } },
            { value: wrapup, name: '后处理', itemStyle: { color: '#fbbf24' } },
            { value: breakaway, name: '小休/离席', itemStyle: { color: '#f87171' } },
            { value: offline, name: '未上线', itemStyle: { color: '#6b7280' } },
          ],
        },
      ],
      graphic: [
        {
          type: 'text',
          left: '28%',
          top: '42%',
          style: {
            text: String(total),
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
            text: '总坐席',
            fontSize: 12,
            fill: tc.muted,
            textAlign: 'center',
          },
        },
      ],
    });
  }, [stats]);

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

  // ─── Agent Filter ───
  const filteredAgents = MOCK_AGENTS.filter((a) => agentFilter === 'all' || a.state === agentFilter);

  // ─── Derived KPI Values ───
  const totalCalls = stats?.today_total_calls ?? 3842;
  const answeredCalls = stats?.today_answered_calls ?? 2516;
  const failedCalls = stats?.today_failed_calls ?? 1326;
  const activeCalls = stats?.active_calls ?? 52;
  const answerRate = stats?.answer_rate ?? 89.4;
  const registeredUsers = stats?.registered_users ?? 120;

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
      <aside className={`cc-sidebar ${sidebarCollapsed ? 'cc-sidebar--collapsed' : ''}`}>
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
          {!sidebarCollapsed && (
            <div className="cc-sidebar__brand-text">
              <span className="cc-sidebar__brand-name">呼叫中心</span>
              <span className="cc-sidebar__brand-sub">管理控制台</span>
            </div>
          )}
        </div>

        <nav className="cc-sidebar__nav">
          {SIDEBAR_GROUPS.map((g) => (
            <div className="cc-sidebar__group" key={g}>
              {!sidebarCollapsed && <div className="cc-sidebar__group-title">{g}</div>}
              {SIDEBAR_ITEMS.filter((it) => it.group === g).map((item) => (
                <div
                  key={item.key}
                  className={`cc-sidebar__item ${item.key === '/dashboard' ? 'is-active' : ''}`}
                  onClick={() => {/* navigate */}}
                >
                  <span className="cc-sidebar__icon">{item.icon}</span>
                  {!sidebarCollapsed && <span className="cc-sidebar__title">{item.title}</span>}
                  {!sidebarCollapsed && item.badge && <span className="cc-sidebar__badge">{item.badge}</span>}
                </div>
              ))}
            </div>
          ))}
        </nav>

        <div className="cc-sidebar__footer">
          <div className="cc-sidebar__user">
            <div className="cc-sidebar__user-avatar">管</div>
            {!sidebarCollapsed && (
              <div className="cc-sidebar__user-info">
                <span className="cc-sidebar__user-name">张管理</span>
                <span className="cc-sidebar__user-role">超级管理员</span>
              </div>
            )}
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
            <button className="cc-header__btn cc-header__btn--danger" title="紧急放音">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                <path d="M11 5L6 9H2v6h4l5 4V5z" />
                <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
              </svg>
              紧急放音
            </button>
          </div>
        </header>

        {/* Content */}
        <main className="cc-content">
          {error && <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />}

          {/* KPI Cards */}
          <div className="cc-kpi-grid">
            <KpiCard
              label="当前在线坐席"
              value={registeredUsers}
              trend={{ value: '12%', direction: 'up' }}
              sub={`总数 ${registeredUsers} · 空闲 24 · 通话 ${activeCalls} · 后处理 10`}
              barColor="var(--accent)"
              barPercent={72}
            />
            <KpiCard
              label="当前排队人数"
              value={17}
              trend={{ value: '8%', direction: 'down' }}
              sub="最长等待 02:34 · 平均等待 00:48"
              barColor="var(--status-break)"
              barPercent={34}
            />
            <KpiCard
              label="服务水平 (SL)"
              value={`${answerRate.toFixed(1)}%`}
              trend={{ value: '3.2%', direction: 'up' }}
              sub="目标 ≥ 85% · 20s内接听"
              barColor="var(--status-online)"
              barPercent={answerRate}
            />
            <KpiCard
              label="今日总话务量"
              value={totalCalls.toLocaleString()}
              trend={{ value: '18%', direction: 'up' }}
              sub={`呼入 ${answeredCalls.toLocaleString()} · 呼出 ${failedCalls.toLocaleString()}`}
              barColor="#3b82f6"
              barPercent={65}
            />
            <KpiCard
              label="平均处理时长"
              value="4:32"
              trend={{ value: '0%', direction: 'flat' }}
              sub="平均通话 3:48 · 后处理 0:44"
              barColor="var(--color-info)"
              barPercent={55}
            />
            <KpiCard
              label="放弃率"
              value="3.8%"
              trend={{ value: '1.2%', direction: 'down' }}
              sub="目标 ≤ 5% · 放弃 96 通"
              barColor="var(--color-danger)"
              barPercent={38}
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

            {/* Queue Distribution */}
            <div className="cc-chart-card cc-chart-card--donut">
              <div className="cc-chart-card__header">
                <h3 className="cc-chart-card__title">队列状态分布</h3>
                <div className="cc-chart-card__tabs">
                  <button className={`cc-tab-btn ${queueFilter === 'all' ? 'cc-tab-btn--active' : ''}`} onClick={() => setQueueFilter('all')}>全部</button>
                  <button className={`cc-tab-btn ${queueFilter === 'skill' ? 'cc-tab-btn--active' : ''}`} onClick={() => setQueueFilter('skill')}>按技能组</button>
                </div>
              </div>
              <div ref={donutRef} className="cc-chart cc-chart--donut" />
            </div>
          </div>

          {/* Agent Status Table */}
          <div className="cc-table-card">
            <div className="cc-table-card__header">
              <h3 className="cc-table-card__title">坐席实时状态</h3>
              <div className="cc-table-card__filters">
                {(['all', 'talking', 'idle', 'wrapup', 'offline'] as const).map((f) => (
                  <button
                    key={f}
                    className={`cc-filter-btn ${agentFilter === f ? 'cc-filter-btn--active' : ''}`}
                    onClick={() => setAgentFilter(f)}
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
                    <th>坐席</th>
                    <th>技能组</th>
                    <th>状态</th>
                    <th>当前通话</th>
                    <th>今日接听</th>
                    <th>今日呼出</th>
                    <th>利用率</th>
                    <th>满意度</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredAgents.map((agent) => {
                    const stateCfg = STATE_CONFIG[agent.state];
                    return (
                      <tr key={agent.id}>
                        <td>
                          <div className="cc-agent-cell">
                            <div className="cc-agent-avatar" style={{ background: SKILL_COLORS[agent.skillGroup] || 'var(--accent)' }}>
                              {agent.name[0]}
                            </div>
                            <div className="cc-agent-info">
                              <span className="cc-agent-name">{agent.name}</span>
                              <span className="cc-agent-id">ID: {agent.agentId}</span>
                            </div>
                          </div>
                        </td>
                        <td>
                          <span className="cc-skill-tag" style={{ color: SKILL_COLORS[agent.skillGroup], borderColor: SKILL_COLORS[agent.skillGroup] }}>
                            {agent.skillGroup}
                          </span>
                        </td>
                        <td>
                          <span className="cc-state-tag" style={{ color: stateCfg.color }}>
                            <span className="cc-state-dot" style={{ background: stateCfg.dot }} />
                            {stateCfg.text}
                          </span>
                        </td>
                        <td>
                          <span className="cc-duration mono" style={{ color: agent.state === 'talking' ? 'var(--status-online)' : agent.state === 'ringing' || agent.state === 'wrapup' ? 'var(--status-break)' : 'var(--text-muted)' }}>
                            {agent.currentCallDuration > 0 ? formatTime(agent.currentCallDuration) : '--'}
                          </span>
                        </td>
                        <td className="mono">{agent.todayAnswered}</td>
                        <td className="mono">{agent.todayOutbound}</td>
                        <td>
                          <div className="cc-utilization">
                            <div className="cc-utilization__bar">
                              <div
                                className="cc-utilization__fill"
                                style={{
                                  width: `${agent.utilization}%`,
                                  background: agent.utilization >= 80 ? 'var(--status-online)' : agent.utilization >= 50 ? 'var(--status-break)' : 'var(--text-muted)',
                                }}
                              />
                            </div>
                            <span className="cc-utilization__text mono" style={{ color: agent.utilization >= 80 ? 'var(--status-online)' : 'var(--text-muted)' }}>
                              {agent.utilization}%
                            </span>
                          </div>
                        </td>
                        <td>
                          <div className="cc-stars">
                            {[1, 2, 3, 4, 5].map((s) => (
                              <span key={s} className={`cc-star ${s <= agent.satisfaction ? 'cc-star--filled' : ''}`}>●</span>
                            ))}
                          </div>
                        </td>
                        <td>
                          <div className="cc-actions">
                            <button className="cc-action-btn" title="监听">
                              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/></svg>
                            </button>
                            <button className="cc-action-btn" title="强插">
                              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"/></svg>
                            </button>
                            <button className="cc-action-btn cc-action-btn--danger" title="强拆">
                              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><line x1="1" y1="1" x2="23" y2="23"/><path d="M16.72 11.06A10.94 10.94 0 0 1 19 12.55"/><path d="M5 12.55a10.94 10.94 0 0 1 5.17-2.39"/><path d="M10.71 5.05A16 16 0 0 1 22.56 9"/><path d="M1.42 9a15.91 15.91 0 0 1 4.7-2.88"/><path d="M8.53 16.11a6 6 0 0 1 6.95 0"/><line x1="12" y1="20" x2="12.01" y2="20"/></svg>
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {filteredAgents.length === 0 && (
                <div className="cc-table-empty">
                  <Empty description="无匹配坐席" />
                </div>
              )}
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}
