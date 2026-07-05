import { useEffect, useState, useRef } from 'react';
import type { ReactNode } from 'react';
import { Card, Grid, Table, Spin, Alert, Button, Empty } from '@arco-design/web-react';
import {
  IconPhone,
  IconCheckCircle,
  IconSound,
  IconUserGroup,
  IconRefresh,
  IconArrowRise,
  IconArrowFall,
} from '@arco-design/web-react/icon';
import * as echarts from 'echarts';
import { apiService } from '@/services/api';
import type { DashboardStats, CdrEvent, HourlyTrend } from '@/types';
import { extractSipUser } from '@/utils/sip';
import './Dashboard.css';

const { Row, Col } = Grid;

function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours > 0) return `${hours}h ${minutes % 60}m`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
  return `${seconds}s`;
}

function StatusTag({ status }: { status: string }) {
  const map: Record<string, string> = {
    answered: 'status-tag status-tag--answered',
    canceled: 'status-tag status-tag--canceled',
    failed: 'status-tag status-tag--failed',
  };
  const text: Record<string, string> = {
    answered: '已接通',
    canceled: '已取消',
    failed: '失败',
  };
  return <span className={map[status] || 'status-tag'}>{text[status] || status}</span>;
}

interface StatCardProps {
  icon: ReactNode;
  label: string;
  value: ReactNode;
  sub: ReactNode;
  gradient: string;
}

function StatCard({ icon, label, value, sub, gradient }: StatCardProps) {
  return (
    <Card className="stat-card app-card" bordered={false}>
      <div className="stat-card__top">
        <div className="stat-card__icon" style={{ background: gradient }}>
          {icon}
        </div>
        <div className="stat-card__label">{label}</div>
      </div>
      <div className="stat-card__value font-num">{value}</div>
      <div className="stat-card__sub">{sub}</div>
    </Card>
  );
}

export default function Dashboard() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [trend, setTrend] = useState<HourlyTrend[]>([]);
  const [recentCdrs, setRecentCdrs] = useState<CdrEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const trendRef = useRef<HTMLDivElement>(null);
  const trendChart = useRef<echarts.ECharts | null>(null);
  const donutRef = useRef<HTMLDivElement>(null);
  const donutChart = useRef<echarts.ECharts | null>(null);

  const loadData = async () => {
    setLoading(true);
    setError(null);
    try {
      const [s, t, c] = await Promise.all([
        apiService.getDashboardStats(),
        apiService.getHourlyTrend(),
        apiService.getRecentCdrs(8),
      ]);
      setStats(s);
      setTrend(t);
      setRecentCdrs(c);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载数据失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  // 趋势折线图
  useEffect(() => {
    if (!trendRef.current || trend.length === 0) return;
    if (!trendChart.current) {
      trendChart.current = echarts.init(trendRef.current);
    }
    const hours = trend.map((t) => `${String(t.hour).padStart(2, '0')}:00`);
    const totals = trend.map((t) => t.total);
    const answered = trend.map((t) => t.answered);

    trendChart.current.setOption({
      tooltip: { trigger: 'axis' },
      legend: { data: ['总呼叫', '已接通'], bottom: 0, icon: 'roundRect' },
      grid: { left: 36, right: 16, top: 20, bottom: 40 },
      xAxis: {
        type: 'category',
        data: hours,
        boundaryGap: false,
        axisLine: { lineStyle: { color: '#e5e6eb' } },
        axisLabel: { color: '#86909c', fontSize: 11 },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: '#f0f1f3' } },
        axisLabel: { color: '#86909c', fontSize: 11 },
      },
      series: [
        {
          name: '总呼叫',
          type: 'line',
          smooth: true,
          symbol: 'circle',
          symbolSize: 6,
          data: totals,
          lineStyle: { color: '#165dff', width: 2.5 },
          itemStyle: { color: '#165dff' },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(22,93,255,0.25)' },
              { offset: 1, color: 'rgba(22,93,255,0.01)' },
            ]),
          },
        },
        {
          name: '已接通',
          type: 'line',
          smooth: true,
          symbol: 'circle',
          symbolSize: 6,
          data: answered,
          lineStyle: { color: '#0fc6c2', width: 2.5 },
          itemStyle: { color: '#0fc6c2' },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(15,198,194,0.22)' },
              { offset: 1, color: 'rgba(15,198,194,0.01)' },
            ]),
          },
        },
      ],
    });
  }, [trend]);

  // 状态分布环图
  useEffect(() => {
    if (!donutRef.current || !stats) return;
    if (!donutChart.current) {
      donutChart.current = echarts.init(donutRef.current);
    }
    donutChart.current.setOption({
      tooltip: { trigger: 'item' },
      legend: { bottom: 0, icon: 'circle', textStyle: { color: '#4e5969', fontSize: 12 } },
      series: [
        {
          type: 'pie',
          radius: ['55%', '78%'],
          center: ['50%', '42%'],
          avoidLabelOverlap: true,
          itemStyle: { borderRadius: 6, borderColor: '#fff', borderWidth: 3 },
          label: { show: false },
          emphasis: { scaleSize: 6 },
          data: [
            { value: stats.today_answered_calls, name: '已接通', itemStyle: { color: '#00b42a' } },
            { value: stats.today_canceled_calls, name: '已取消', itemStyle: { color: '#ff7d00' } },
            { value: stats.today_failed_calls, name: '失败', itemStyle: { color: '#f53f3f' } },
          ],
        },
      ],
      graphic: [
        {
          type: 'text',
          left: 'center',
          top: '38%',
          style: {
            text: String(stats.today_total_calls),
            fontSize: 26,
            fontWeight: 700,
            fill: '#1d2129',
            fontFamily: 'JetBrains Mono, monospace',
          },
        },
        {
          type: 'text',
          left: 'center',
          top: '50%',
          style: { text: '今日呼叫', fontSize: 12, fill: '#86909c' },
        },
      ],
    });
  }, [stats]);

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

  const columns = [
    { title: '呼叫 ID', dataIndex: 'call_id', width: 200, ellipsis: true },
    {
      title: '主叫',
      dataIndex: 'caller',
      width: 160,
      render: (c: string) => <span className="cell-mono">{extractSipUser(c)}</span>,
    },
    { title: '被叫', dataIndex: 'callee', width: 140 },
    {
      title: '状态',
      dataIndex: 'status',
      width: 100,
      render: (s: string) => <StatusTag status={s} />,
    },
    {
      title: '时长',
      dataIndex: 'duration_ms',
      width: 110,
      render: (ms: number) => <span className="font-num">{formatDuration(ms)}</span>,
    },
    {
      title: 'MOS',
      dataIndex: 'mos',
      width: 80,
      render: (m: number) => (m ? <span className="font-num">{m.toFixed(1)}</span> : '-'),
    },
  ];

  if (loading) {
    return (
      <div className="loading-wrap">
        <Spin size={32} />
        <span>正在加载监控数据…</span>
      </div>
    );
  }

  return (
    <div className="page-wrap dashboard">
      <div className="page-header">
        <div className="page-header__title">
          <h1>运营监控大盘</h1>
          <span className="sub">实时掌握平台呼叫、质量与设备状态</span>
        </div>
        <div className="page-header__actions">
          <Button icon={<IconRefresh />} onClick={loadData}>
            刷新
          </Button>
        </div>
      </div>

      {error && (
        <Alert type="error" content={error} closable style={{ marginBottom: 16 }} />
      )}

      {stats && (
        <>
          <Row gutter={[16, 16]}>
            <Col span={6}>
              <StatCard
                icon={<IconPhone />}
                label="活跃呼叫"
                value={stats.active_calls}
                sub={<span className="stat-card__live">● 实时通话中</span>}
                gradient="linear-gradient(135deg, #4080FF 0%, #165DFF 100%)"
              />
            </Col>
            <Col span={6}>
              <StatCard
                icon={<IconCheckCircle />}
                label="今日呼叫"
                value={stats.today_total_calls}
                sub={
                  <span className="stat-card__split">
                    <em style={{ color: '#00b42a' }}>{stats.today_answered_calls}</em>接通
                    <em style={{ color: '#ff7d00' }}>{stats.today_canceled_calls}</em>取消
                    <em style={{ color: '#f53f3f' }}>{stats.today_failed_calls}</em>失败
                  </span>
                }
                gradient="linear-gradient(135deg, #0FC6C2 0%, #0AA8A4 100%)"
              />
            </Col>
            <Col span={6}>
              <StatCard
                icon={<IconSound />}
                label="接通率"
                value={<>{stats.answer_rate.toFixed(1)}<span className="stat-card__unit">%</span></>}
                sub={
                  <span className="stat-card__trend">
                    {stats.answer_rate >= 80 ? (
                      <IconArrowRise style={{ color: '#00b42a' }} />
                    ) : (
                      <IconArrowFall style={{ color: '#f53f3f' }} />
                    )}
                    {stats.answer_rate >= 80 ? '表现良好' : '需关注'}
                  </span>
                }
                gradient="linear-gradient(135deg, #23C343 0%, #00B42A 100%)"
              />
            </Col>
            <Col span={6}>
              <StatCard
                icon={<IconUserGroup />}
                label="注册终端"
                value={stats.registered_users}
                sub={
                  <span className="stat-card__split">
                    <em style={{ color: '#165dff' }}>{stats.active_gateways}</em>个网关在线
                  </span>
                }
                gradient="linear-gradient(135deg, #7BC2FF 0%, #722ED1 100%)"
              />
            </Col>
          </Row>

          <Row gutter={[16, 16]} style={{ marginTop: 16 }}>
            <Col span={16}>
              <Card className="app-card" bordered={false} title="今日呼叫趋势（按小时）">
                <div ref={trendRef} className="trend-chart" />
                {trend.length === 0 && (
                  <div className="chart-empty">
                    <Empty description="今日暂无呼叫数据" />
                  </div>
                )}
              </Card>
            </Col>
            <Col span={8}>
              <Card className="app-card" bordered={false} title="呼叫状态分布">
                <div ref={donutRef} className="donut-chart" />
              </Card>
            </Col>
          </Row>

          <Row gutter={[16, 16]} style={{ marginTop: 16 }}>
            <Col span={8}>
              <Card className="app-card quality-card" bordered={false}>
                <div className="quality-card__head">
                  <span>平均 MOS</span>
                  <span className={`quality-grade ${mosGrade(stats.avg_mos).cls}`}>
                    {mosGrade(stats.avg_mos).text}
                  </span>
                </div>
                <div className="quality-card__value font-num">
                  {stats.avg_mos ? stats.avg_mos.toFixed(2) : '—'}
                </div>
                <div className="quality-bar">
                  <div
                    className="quality-bar__fill"
                    style={{
                      width: `${stats.avg_mos ? (stats.avg_mos / 5) * 100 : 0}%`,
                      background: 'linear-gradient(90deg,#165dff,#0fc6c2)',
                    }}
                  />
                </div>
                <div className="quality-card__foot">语音质量评分（0-5，越高越好）</div>
              </Card>
            </Col>
            <Col span={8}>
              <Card className="app-card quality-card" bordered={false}>
                <div className="quality-card__head">
                  <span>平均丢包率</span>
                  <span className={`quality-grade ${lossGrade(stats.avg_loss_rate).cls}`}>
                    {lossGrade(stats.avg_loss_rate).text}
                  </span>
                </div>
                <div className="quality-card__value font-num">
                  {stats.avg_loss_rate != null ? `${stats.avg_loss_rate.toFixed(3)}%` : '—'}
                </div>
                <div className="quality-bar">
                  <div
                    className="quality-bar__fill"
                    style={{
                      width: `${Math.min(100, (stats.avg_loss_rate || 0) * 50)}%`,
                      background: 'linear-gradient(90deg,#00b42a,#ff7d00,#f53f3f)',
                    }}
                  />
                </div>
                <div className="quality-card__foot">RTCP 上报丢包比例</div>
              </Card>
            </Col>
            <Col span={8}>
              <Card className="app-card quality-card" bordered={false}>
                <div className="quality-card__head">
                  <span>平均抖动</span>
                  <span className={`quality-grade ${jitterGrade(stats.avg_jitter_ms).cls}`}>
                    {jitterGrade(stats.avg_jitter_ms).text}
                  </span>
                </div>
                <div className="quality-card__value font-num">
                  {stats.avg_jitter_ms != null ? `${stats.avg_jitter_ms.toFixed(1)} ms` : '—'}
                </div>
                <div className="quality-bar">
                  <div
                    className="quality-bar__fill"
                    style={{
                      width: `${Math.min(100, (stats.avg_jitter_ms || 0) * 5)}%`,
                      background: 'linear-gradient(90deg,#00b42a,#ff7d00,#f53f3f)',
                    }}
                  />
                </div>
                <div className="quality-card__foot">网络抖动（ms，越低越好）</div>
              </Card>
            </Col>
          </Row>

          <Card
            className="app-card"
            bordered={false}
            title="最近呼叫"
            style={{ marginTop: 16 }}
          >
            <Table
              className="app-table"
              columns={columns}
              data={recentCdrs}
              rowKey="call_id"
              pagination={false}
              noDataElement={<Empty description="暂无呼叫记录" />}
            />
          </Card>
        </>
      )}
    </div>
  );
}

function mosGrade(v?: number) {
  if (v == null) return { text: '无数据', cls: 'g-muted' };
  if (v >= 4) return { text: '优秀', cls: 'g-good' };
  if (v >= 3.5) return { text: '良好', cls: 'g-ok' };
  return { text: '较差', cls: 'g-bad' };
}
function lossGrade(v?: number) {
  if (v == null) return { text: '无数据', cls: 'g-muted' };
  if (v < 1) return { text: '优秀', cls: 'g-good' };
  if (v < 3) return { text: '良好', cls: 'g-ok' };
  return { text: '较差', cls: 'g-bad' };
}
function jitterGrade(v?: number) {
  if (v == null) return { text: '无数据', cls: 'g-muted' };
  if (v < 20) return { text: '优秀', cls: 'g-good' };
  if (v < 40) return { text: '良好', cls: 'g-ok' };
  return { text: '较差', cls: 'g-bad' };
}
